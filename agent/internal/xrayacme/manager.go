// Package xrayacme provides Honey-managed ACME certificates for Xray.
//
// Xray consumes certificate/key files but does not own an ACME client. This
// manager runs beside it, answers HTTP-01 behind Caddy, persists ACME state,
// exports PEM files atomically and asks the agent to reload Xray after renewal.
package xrayacme

import (
	"context"
	"crypto"
	"crypto/ecdsa"
	"crypto/elliptic"
	"crypto/rand"
	"crypto/sha256"
	"crypto/tls"
	"crypto/x509"
	"crypto/x509/pkix"
	"encoding/hex"
	"encoding/json"
	"encoding/pem"
	"fmt"
	"io"
	"math/big"
	"net"
	"net/http"
	"net/http/httputil"
	"net/url"
	"os"
	"path/filepath"
	"sort"
	"strconv"
	"strings"
	"sync"
	"time"

	"golang.org/x/crypto/acme"
	"golang.org/x/crypto/acme/autocert"

	"github.com/akiko99x/honey/agent/internal/core"
	"github.com/akiko99x/honey/agent/internal/logx"
)

const (
	DefaultListen          = "127.0.0.1:9080"
	DefaultSingboxUpstream = "127.0.0.1:9082"
	DefaultRoot            = "/etc/honey/xray/acme"
	stagingDirectory       = "https://acme-staging-v02.api.letsencrypt.org/directory"
)

type acmeConfig struct {
	Email                   string `json:"email"`
	CA                      string `json:"ca"`
	DirectoryURL            string `json:"directory_url"`
	AlternativeHTTPPort     int    `json:"alternative_http_port"`
	DisableHTTPChallenge    bool   `json:"disable_http_challenge"`
	DisableTLSALPNChallenge bool   `json:"disable_tls_alpn_challenge"`
}

type request struct {
	domain    string
	email     string
	directory string
}

type persistedRequest struct {
	Domain    string `json:"domain"`
	Email     string `json:"email"`
	Directory string `json:"directory"`
}

type managedDomain struct {
	request
	client     *acme.Client
	account    *acme.Account
	challenges map[string]string
	mu         sync.Mutex
	challenge  sync.RWMutex
}

// Manager owns the local challenge gateway and active Xray ACME domains.
type Manager struct {
	root            string
	listen          string
	singboxUpstream string

	mu      sync.RWMutex
	domains map[string]*managedDomain
	server  *http.Server
	proxy   http.Handler
	reload  func() error
}

func New(root, listen, singboxUpstream string) *Manager {
	if root == "" {
		root = DefaultRoot
	}
	if listen == "" {
		listen = DefaultListen
	}
	if singboxUpstream == "" {
		singboxUpstream = DefaultSingboxUpstream
	}
	target, _ := url.Parse("http://" + singboxUpstream)
	return &Manager{
		root:            root,
		listen:          listen,
		singboxUpstream: singboxUpstream,
		domains:         map[string]*managedDomain{},
		proxy:           httputil.NewSingleHostReverseProxy(target),
	}
}

// SetReload installs the callback used after a renewed certificate was
// atomically exported. The callback should re-apply Xray's persisted config.
func (m *Manager) SetReload(fn func() error) {
	m.mu.Lock()
	m.reload = fn
	m.mu.Unlock()
}

// Start reserves the local HTTP-01 gateway before any configuration can be
// pushed. Unknown hosts are forwarded to sing-box's internal ACME listener.
func (m *Manager) Start(ctx context.Context) error {
	persisted, err := m.loadRequests()
	if err != nil {
		return fmt.Errorf("load Xray ACME registrations: %w", err)
	}
	if err := m.activate(persisted); err != nil {
		return fmt.Errorf("restore Xray ACME registrations: %w", err)
	}
	listener, err := net.Listen("tcp", m.listen)
	if err != nil {
		return fmt.Errorf("listen on xray ACME gateway %s: %w", m.listen, err)
	}
	m.server = &http.Server{
		Handler:           m,
		ReadHeaderTimeout: 10 * time.Second,
		IdleTimeout:       30 * time.Second,
	}
	go func() {
		<-ctx.Done()
		shutdown, cancel := context.WithTimeout(context.Background(), 5*time.Second)
		defer cancel()
		_ = m.server.Shutdown(shutdown)
	}()
	go func() {
		if err := m.server.Serve(listener); err != nil && err != http.ErrServerClosed {
			logx.Error(logx.AgentACMEGateway, "Xray ACME gateway stopped: %v", err)
		}
	}()
	go m.renewLoop(ctx)
	logx.Info(logx.AgentACMEGateway, "Xray ACME HTTP-01 gateway listening at %s", m.listen)
	return nil
}

func (m *Manager) ServeHTTP(w http.ResponseWriter, r *http.Request) {
	host := strings.ToLower(r.Host)
	if parsedHost, _, err := net.SplitHostPort(host); err == nil {
		host = parsedHost
	}
	m.mu.RLock()
	domain := m.domains[host]
	m.mu.RUnlock()
	if domain != nil {
		const prefix = "/.well-known/acme-challenge/"
		if strings.HasPrefix(r.URL.Path, prefix) {
			token := strings.TrimPrefix(r.URL.Path, prefix)
			domain.challenge.RLock()
			response, ok := domain.challenges[token]
			domain.challenge.RUnlock()
			if ok {
				w.Header().Set("Content-Type", "text/plain")
				_, _ = io.WriteString(w, response)
				return
			}
		}
	}
	m.proxy.ServeHTTP(w, r)
}

// Prepare injects certificate paths into an Xray desired-state spec. Apply
// mode obtains/renews real certificates; validation mode uses existing files
// or a disposable self-signed pair so Xray can validate without side effects.
func (m *Manager) Prepare(ctx context.Context, spec *core.Spec, issue bool) (func(), error) {
	requests, singboxDomains, err := m.requests(spec)
	if err != nil {
		return func() {}, err
	}
	for domain := range requests {
		if singboxDomains[domain] {
			return func() {}, fmt.Errorf(
				"domain %q cannot use sing-box and Xray ACME simultaneously; use one managed certificate source",
				domain,
			)
		}
	}

	m.rewriteSingboxPorts(spec)
	if issue {
		if err := m.activate(requests); err != nil {
			return func() {}, err
		}
		for domain := range requests {
			if _, err := m.ensure(ctx, domain); err != nil {
				return func() {}, fmt.Errorf("issue Xray certificate for %s: %w", domain, err)
			}
		}
		m.injectPaths(spec, requests, nil)
		return func() {}, nil
	}

	var tempDirs []string
	temporary := map[string][2]string{}
	for domain := range requests {
		certPath, keyPath := m.paths(domain)
		if regular(certPath) && regular(keyPath) {
			continue
		}
		dir, cert, key, err := temporaryCertificate(domain)
		if err != nil {
			return func() {}, err
		}
		tempDirs = append(tempDirs, dir)
		temporary[domain] = [2]string{cert, key}
	}
	m.injectPaths(spec, requests, temporary)
	return func() {
		for _, dir := range tempDirs {
			_ = os.RemoveAll(dir)
		}
	}, nil
}

// InjectPaths is used by drift reporting: it is deterministic and never
// performs network or filesystem writes.
func (m *Manager) InjectPaths(spec *core.Spec) error {
	requests, _, err := m.requests(spec)
	if err != nil {
		return err
	}
	m.rewriteSingboxPorts(spec)
	m.injectPaths(spec, requests, nil)
	return nil
}

func (m *Manager) requests(spec *core.Spec) (map[string]request, map[string]bool, error) {
	requests := map[string]request{}
	singbox := map[string]bool{}
	for _, inbound := range spec.Inbounds {
		cfg, enabled, err := parseConfig(inbound.ExtraJSON)
		if err != nil {
			return nil, nil, fmt.Errorf("inbound %q ACME: %w", inbound.Tag, err)
		}
		if !enabled {
			continue
		}
		if inbound.TLS == nil || !inbound.TLS.Enabled || inbound.TLS.Reality != nil {
			return nil, nil, fmt.Errorf("inbound %q ACME requires ordinary TLS", inbound.Tag)
		}
		domain, err := validDomain(inbound.TLS.ServerName)
		if err != nil {
			return nil, nil, fmt.Errorf("inbound %q ACME: %w", inbound.Tag, err)
		}
		if inbound.Core != "xray" {
			singbox[domain] = true
			continue
		}
		if cfg.DisableHTTPChallenge {
			return nil, nil, fmt.Errorf("Xray ACME supports HTTP-01 only")
		}
		if cfg.Email == "" || !strings.Contains(cfg.Email, "@") {
			return nil, nil, fmt.Errorf("Xray ACME requires a contact email")
		}
		if cfg.AlternativeHTTPPort != 0 && cfg.AlternativeHTTPPort != listenPort(m.listen) {
			return nil, nil, fmt.Errorf(
				"Xray ACME alternative_http_port must be %d (the Honey challenge gateway)",
				listenPort(m.listen),
			)
		}
		directory, err := directoryURL(cfg)
		if err != nil {
			return nil, nil, err
		}
		next := request{domain: domain, email: strings.TrimSpace(cfg.Email), directory: directory}
		if current, exists := requests[domain]; exists && current != next {
			return nil, nil, fmt.Errorf("domain %q has conflicting ACME settings", domain)
		}
		requests[domain] = next
	}
	return requests, singbox, nil
}

func parseConfig(raw json.RawMessage) (acmeConfig, bool, error) {
	if len(raw) == 0 {
		return acmeConfig{}, false, nil
	}
	var extra map[string]json.RawMessage
	if err := json.Unmarshal(raw, &extra); err != nil {
		return acmeConfig{}, false, err
	}
	value, ok := extra["acme"]
	if !ok || string(value) == "null" || string(value) == "false" {
		return acmeConfig{}, false, nil
	}
	if string(value) == "true" {
		return acmeConfig{}, true, nil
	}
	var cfg acmeConfig
	if err := json.Unmarshal(value, &cfg); err != nil {
		return acmeConfig{}, false, err
	}
	return cfg, true, nil
}

func directoryURL(cfg acmeConfig) (string, error) {
	if cfg.DirectoryURL != "" {
		u, err := url.Parse(cfg.DirectoryURL)
		if err != nil || u.Scheme != "https" || u.Host == "" {
			return "", fmt.Errorf("ACME directory_url must be an https URL")
		}
		return cfg.DirectoryURL, nil
	}
	switch strings.ToLower(strings.TrimSpace(cfg.CA)) {
	case "", "letsencrypt", "production":
		return autocert.DefaultACMEDirectory, nil
	case "staging", "letsencrypt-staging":
		return stagingDirectory, nil
	default:
		return "", fmt.Errorf("unsupported ACME CA %q", cfg.CA)
	}
}

func (m *Manager) activate(requests map[string]request) error {
	next := make(map[string]*managedDomain, len(requests))
	m.mu.Lock()
	defer m.mu.Unlock()
	for domain, req := range requests {
		if current := m.domains[domain]; current != nil && current.request == req {
			next[domain] = current
			continue
		}
		// Keep staging and production account keys separate. Switching CA
		// directories must not reuse an account KID from the other CA.
		cacheDir := filepath.Join(m.root, safePart(domain+"|"+req.directory), "cache")
		if err := os.MkdirAll(cacheDir, 0o700); err != nil {
			return err
		}
		key, account, err := loadAccount(cacheDir, req.email)
		if err != nil {
			return fmt.Errorf("load ACME account for %s: %w", domain, err)
		}
		client := &acme.Client{Key: key, DirectoryURL: req.directory}
		if account.URI != "" {
			client.KID = acme.KeyID(account.URI)
		}
		next[domain] = &managedDomain{
			request: req, client: client, account: account,
			challenges: map[string]string{},
		}
	}
	m.domains = next
	return m.persistRequests(requests)
}

func (m *Manager) registrationsPath() string {
	return filepath.Join(m.root, "registrations.json")
}

func (m *Manager) loadRequests() (map[string]request, error) {
	data, err := os.ReadFile(m.registrationsPath())
	if os.IsNotExist(err) {
		return map[string]request{}, nil
	}
	if err != nil {
		return nil, err
	}
	var stored []persistedRequest
	if err := json.Unmarshal(data, &stored); err != nil {
		return nil, err
	}
	requests := make(map[string]request, len(stored))
	for _, item := range stored {
		domain, err := validDomain(item.Domain)
		if err != nil {
			return nil, err
		}
		if item.Email == "" || item.Directory == "" {
			return nil, fmt.Errorf("incomplete registration for %s", domain)
		}
		requests[domain] = request{domain: domain, email: item.Email, directory: item.Directory}
	}
	return requests, nil
}

func (m *Manager) persistRequests(requests map[string]request) error {
	stored := make([]persistedRequest, 0, len(requests))
	for _, item := range requests {
		stored = append(stored, persistedRequest{
			Domain: item.domain, Email: item.email, Directory: item.directory,
		})
	}
	sort.Slice(stored, func(i, j int) bool { return stored[i].Domain < stored[j].Domain })
	data, err := json.MarshalIndent(stored, "", "  ")
	if err != nil {
		return err
	}
	data = append(data, '\n')
	return atomicWrite(m.registrationsPath(), data, 0o600)
}

func (m *Manager) ensure(ctx context.Context, domain string) (bool, error) {
	m.mu.RLock()
	entry := m.domains[domain]
	m.mu.RUnlock()
	if entry == nil {
		return false, fmt.Errorf("domain is not active")
	}
	entry.mu.Lock()
	defer entry.mu.Unlock()

	certPath, keyPath := m.paths(domain)
	if cert := loadUsableCertificate(certPath, keyPath); cert != nil {
		return false, nil
	}
	cert, err := m.issueHTTP01(ctx, entry)
	if err != nil {
		return false, err
	}
	return m.export(domain, cert)
}

func (m *Manager) issueHTTP01(ctx context.Context, entry *managedDomain) (*tls.Certificate, error) {
	if entry.account == nil || entry.account.URI == "" {
		account, err := entry.client.Register(ctx, &acme.Account{
			Contact: []string{"mailto:" + entry.email},
		}, acme.AcceptTOS)
		if err != nil {
			return nil, fmt.Errorf("register ACME account: %w", err)
		}
		entry.account = account
		if err := saveAccount(filepath.Join(m.root, safePart(entry.domain), "cache"), account); err != nil {
			return nil, fmt.Errorf("save ACME account: %w", err)
		}
	}
	order, err := entry.client.AuthorizeOrder(ctx, acme.DomainIDs(entry.domain))
	if err != nil {
		return nil, fmt.Errorf("create ACME order: %w", err)
	}
	for _, authzURL := range order.AuthzURLs {
		authz, err := entry.client.GetAuthorization(ctx, authzURL)
		if err != nil {
			return nil, fmt.Errorf("fetch ACME authorization: %w", err)
		}
		if authz.Status == acme.StatusValid {
			continue
		}
		var challenge *acme.Challenge
		for _, candidate := range authz.Challenges {
			if candidate.Type == "http-01" {
				challenge = candidate
				break
			}
		}
		if challenge == nil {
			return nil, fmt.Errorf("ACME server did not offer HTTP-01 for %s", entry.domain)
		}
		response, err := entry.client.HTTP01ChallengeResponse(challenge.Token)
		if err != nil {
			return nil, fmt.Errorf("build HTTP-01 response: %w", err)
		}
		entry.challenge.Lock()
		entry.challenges[challenge.Token] = response
		entry.challenge.Unlock()
		_, acceptErr := entry.client.Accept(ctx, challenge)
		if acceptErr == nil {
			_, acceptErr = entry.client.WaitAuthorization(ctx, authz.URI)
		}
		entry.challenge.Lock()
		delete(entry.challenges, challenge.Token)
		entry.challenge.Unlock()
		if acceptErr != nil {
			return nil, fmt.Errorf("validate HTTP-01 challenge: %w", acceptErr)
		}
	}
	order, err = entry.client.WaitOrder(ctx, order.URI)
	if err != nil {
		return nil, fmt.Errorf("wait for ACME order: %w", err)
	}
	key, err := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
	if err != nil {
		return nil, fmt.Errorf("generate certificate key: %w", err)
	}
	csrDER, err := x509.CreateCertificateRequest(rand.Reader, &x509.CertificateRequest{
		Subject:  pkix.Name{CommonName: entry.domain},
		DNSNames: []string{entry.domain},
	}, key)
	if err != nil {
		return nil, fmt.Errorf("create certificate request: %w", err)
	}
	der, _, err := entry.client.CreateOrderCert(ctx, order.FinalizeURL, csrDER, true)
	if err != nil {
		return nil, fmt.Errorf("finalize ACME order: %w", err)
	}
	cert := &tls.Certificate{Certificate: der, PrivateKey: key}
	if len(cert.Certificate) == 0 {
		return nil, fmt.Errorf("ACME returned an empty certificate chain")
	}
	return cert, nil
}

func (m *Manager) export(domain string, cert *tls.Certificate) (bool, error) {
	if cert == nil || len(cert.Certificate) == 0 || cert.PrivateKey == nil {
		return false, fmt.Errorf("ACME returned an incomplete certificate")
	}
	var certPEM []byte
	for _, der := range cert.Certificate {
		certPEM = append(certPEM, pem.EncodeToMemory(&pem.Block{Type: "CERTIFICATE", Bytes: der})...)
	}
	keyDER, err := x509.MarshalPKCS8PrivateKey(cert.PrivateKey)
	if err != nil {
		return false, fmt.Errorf("marshal ACME private key: %w", err)
	}
	keyPEM := pem.EncodeToMemory(&pem.Block{Type: "PRIVATE KEY", Bytes: keyDER})
	certPath, keyPath := m.paths(domain)
	old := digestFiles(certPath, keyPath)
	if err := atomicWrite(certPath, certPEM, 0o600); err != nil {
		return false, err
	}
	if err := atomicWrite(keyPath, keyPEM, 0o600); err != nil {
		return false, err
	}
	return old != digestFiles(certPath, keyPath), nil
}

func loadAccount(cacheDir, email string) (crypto.Signer, *acme.Account, error) {
	keyPath := filepath.Join(cacheDir, "account.key")
	accountPath := filepath.Join(cacheDir, "account.json")
	var key crypto.Signer
	data, err := os.ReadFile(keyPath)
	if os.IsNotExist(err) {
		generated, genErr := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
		if genErr != nil {
			return nil, nil, genErr
		}
		key = generated
		keyDER, marshalErr := x509.MarshalPKCS8PrivateKey(key)
		if marshalErr != nil {
			return nil, nil, marshalErr
		}
		if err := atomicWrite(keyPath, pem.EncodeToMemory(&pem.Block{
			Type: "PRIVATE KEY", Bytes: keyDER,
		}), 0o600); err != nil {
			return nil, nil, err
		}
	} else if err != nil {
		return nil, nil, err
	} else {
		block, _ := pem.Decode(data)
		if block == nil {
			return nil, nil, fmt.Errorf("invalid account key PEM")
		}
		signer, parseErr := x509.ParsePKCS8PrivateKey(block.Bytes)
		if parseErr != nil {
			return nil, nil, parseErr
		}
		var ok bool
		key, ok = signer.(crypto.Signer)
		if !ok {
			return nil, nil, fmt.Errorf("account key is not a signing key")
		}
	}
	var account acme.Account
	data, err = os.ReadFile(accountPath)
	if os.IsNotExist(err) {
		account.Contact = []string{"mailto:" + strings.TrimSpace(email)}
	} else if err != nil {
		return nil, nil, err
	} else if err := json.Unmarshal(data, &account); err != nil {
		return nil, nil, err
	}
	return key, &account, nil
}

func saveAccount(cacheDir string, account *acme.Account) error {
	data, err := json.MarshalIndent(account, "", "  ")
	if err != nil {
		return err
	}
	return atomicWrite(filepath.Join(cacheDir, "account.json"), append(data, '\n'), 0o600)
}

func loadUsableCertificate(certPath, keyPath string) *tls.Certificate {
	cert, err := tls.LoadX509KeyPair(certPath, keyPath)
	if err != nil || len(cert.Certificate) == 0 {
		return nil
	}
	leaf, err := x509.ParseCertificate(cert.Certificate[0])
	if err != nil || time.Until(leaf.NotAfter) <= 30*24*time.Hour {
		return nil
	}
	return &cert
}

func (m *Manager) renewLoop(ctx context.Context) {
	ticker := time.NewTicker(12 * time.Hour)
	defer ticker.Stop()
	for {
		select {
		case <-ctx.Done():
			return
		case <-ticker.C:
			m.mu.RLock()
			domains := make([]string, 0, len(m.domains))
			for domain := range m.domains {
				domains = append(domains, domain)
			}
			reload := m.reload
			m.mu.RUnlock()
			changed := false
			for _, domain := range domains {
				renewCtx, cancel := context.WithTimeout(ctx, 5*time.Minute)
				updated, err := m.ensure(renewCtx, domain)
				cancel()
				if err != nil {
					logx.Warn(logx.AgentACMERenew, "Xray ACME renewal failed for %s: %v", domain, err)
				} else if updated {
					changed = true
					logx.Info(logx.AgentACMERenew, "Xray ACME certificate renewed for %s", domain)
				}
			}
			if changed && reload != nil {
				if err := reload(); err != nil {
					logx.Error(logx.AgentACMERenew, "Xray reload after certificate renewal failed: %v", err)
				}
			}
		}
	}
}

func (m *Manager) injectPaths(spec *core.Spec, requests map[string]request, temporary map[string][2]string) {
	for i := range spec.Inbounds {
		inbound := &spec.Inbounds[i]
		if inbound.Core != "xray" || inbound.TLS == nil {
			continue
		}
		domain := strings.ToLower(strings.TrimSpace(inbound.TLS.ServerName))
		if _, ok := requests[domain]; !ok {
			continue
		}
		if pair, ok := temporary[domain]; ok {
			inbound.TLS.CertPath, inbound.TLS.KeyPath = pair[0], pair[1]
		} else {
			inbound.TLS.CertPath, inbound.TLS.KeyPath = m.paths(domain)
		}
	}
}

func (m *Manager) rewriteSingboxPorts(spec *core.Spec) {
	gateway := listenPort(m.listen)
	upstream := listenPort(m.singboxUpstream)
	for i := range spec.Inbounds {
		inbound := &spec.Inbounds[i]
		if inbound.Core == "xray" || len(inbound.ExtraJSON) == 0 {
			continue
		}
		var extra map[string]any
		if json.Unmarshal(inbound.ExtraJSON, &extra) != nil {
			continue
		}
		acmeValue, ok := extra["acme"]
		if !ok {
			continue
		}
		acmeMap, ok := acmeValue.(map[string]any)
		if !ok {
			continue
		}
		port, _ := acmeMap["alternative_http_port"].(float64)
		if int(port) != gateway {
			continue
		}
		acmeMap["alternative_http_port"] = upstream
		extra["acme"] = acmeMap
		if raw, err := json.Marshal(extra); err == nil {
			inbound.ExtraJSON = raw
		}
	}
}

func (m *Manager) paths(domain string) (string, string) {
	dir := filepath.Join(m.root, safePart(domain))
	return filepath.Join(dir, "fullchain.pem"), filepath.Join(dir, "privkey.pem")
}

func temporaryCertificate(domain string) (string, string, string, error) {
	dir, err := os.MkdirTemp("", "honey-xray-acme-validate-")
	if err != nil {
		return "", "", "", err
	}
	key, err := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
	if err != nil {
		_ = os.RemoveAll(dir)
		return "", "", "", err
	}
	serial, _ := rand.Int(rand.Reader, new(big.Int).Lsh(big.NewInt(1), 120))
	now := time.Now()
	template := x509.Certificate{
		SerialNumber: serial,
		Subject:      pkix.Name{CommonName: domain},
		DNSNames:     []string{domain},
		NotBefore:    now.Add(-time.Minute),
		NotAfter:     now.Add(time.Hour),
		KeyUsage:     x509.KeyUsageDigitalSignature | x509.KeyUsageKeyEncipherment,
		ExtKeyUsage:  []x509.ExtKeyUsage{x509.ExtKeyUsageServerAuth},
	}
	der, err := x509.CreateCertificate(rand.Reader, &template, &template, key.Public(), key)
	if err != nil {
		_ = os.RemoveAll(dir)
		return "", "", "", err
	}
	keyDER, _ := x509.MarshalPKCS8PrivateKey(key)
	certPath, keyPath := filepath.Join(dir, "fullchain.pem"), filepath.Join(dir, "privkey.pem")
	if err := os.WriteFile(certPath, pem.EncodeToMemory(&pem.Block{Type: "CERTIFICATE", Bytes: der}), 0o600); err != nil {
		_ = os.RemoveAll(dir)
		return "", "", "", err
	}
	if err := os.WriteFile(keyPath, pem.EncodeToMemory(&pem.Block{Type: "PRIVATE KEY", Bytes: keyDER}), 0o600); err != nil {
		_ = os.RemoveAll(dir)
		return "", "", "", err
	}
	return dir, certPath, keyPath, nil
}

func validDomain(value string) (string, error) {
	domain := strings.ToLower(strings.TrimSpace(value))
	if domain == "" || net.ParseIP(domain) != nil || strings.ContainsAny(domain, "/\\: \t\r\n") {
		return "", fmt.Errorf("server_name must be a DNS hostname")
	}
	return domain, nil
}

func safePart(value string) string {
	sum := sha256.Sum256([]byte(value))
	base := strings.Map(func(r rune) rune {
		if (r >= 'a' && r <= 'z') || (r >= '0' && r <= '9') || r == '.' || r == '-' {
			return r
		}
		return '-'
	}, strings.ToLower(value))
	return base + "-" + hex.EncodeToString(sum[:4])
}

func listenPort(address string) int {
	_, raw, err := net.SplitHostPort(address)
	if err != nil {
		return 0
	}
	port, _ := strconv.Atoi(raw)
	return port
}

func regular(path string) bool {
	info, err := os.Stat(path)
	return err == nil && info.Mode().IsRegular() && info.Size() > 0
}

func atomicWrite(path string, data []byte, mode os.FileMode) error {
	if err := os.MkdirAll(filepath.Dir(path), 0o700); err != nil {
		return err
	}
	tmp, err := os.CreateTemp(filepath.Dir(path), ".honey-acme-*")
	if err != nil {
		return err
	}
	name := tmp.Name()
	defer os.Remove(name)
	if err := tmp.Chmod(mode); err != nil {
		tmp.Close()
		return err
	}
	if _, err := tmp.Write(data); err != nil {
		tmp.Close()
		return err
	}
	if err := tmp.Sync(); err != nil {
		tmp.Close()
		return err
	}
	if err := tmp.Close(); err != nil {
		return err
	}
	return os.Rename(name, path)
}

func digestFiles(paths ...string) string {
	hash := sha256.New()
	for _, path := range paths {
		data, err := os.ReadFile(path)
		if err != nil {
			return ""
		}
		_, _ = hash.Write(data)
	}
	return hex.EncodeToString(hash.Sum(nil))
}
