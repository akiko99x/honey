// honey-enroll performs the one-time, CSR-based node bootstrap. The private
// key is generated and stored locally; only the CSR is sent to master.
package main

import (
	"bytes"
	"crypto/rand"
	"crypto/rsa"
	"crypto/tls"
	"crypto/x509"
	"crypto/x509/pkix"
	"encoding/json"
	"encoding/pem"
	"errors"
	"flag"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"os"
	"path/filepath"
	"strings"
	"time"

	"github.com/akiko99x/honey/agent/internal/logx"
)

type claimResponse struct {
	NodeID            string `json:"node_id"`
	NodeName          string `json:"node_name"`
	Transport         string `json:"transport"`
	TLSServerName     string `json:"tls_server_name"`
	CertificatePEM    string `json:"certificate_pem"`
	CAPEM             string `json:"ca_pem"`
	SerialNumber      string `json:"serial_number"`
	FingerprintSHA256 string `json:"fingerprint_sha256"`
	ExpiresAt         string `json:"expires_at"`
}

func main() {
	master := flag.String("master", "", "master URL, e.g. https://panel.example.com")
	token := flag.String("token", "", "one-time enrollment token")
	certsDir := flag.String("certs-dir", "/etc/honey/certs", "certificate output directory")
	envFile := flag.String("env-file", "/etc/honey/agent.env", "agent environment file")
	masterCA := flag.String("master-ca", "", "optional CA file for a private master HTTPS certificate")
	masterAddr := flag.String("master-addr", "", "dial acceptor host:port for dial/both nodes")
	listen := flag.String("listen", "0.0.0.0:8443", "agent gRPC listen address for serve/both nodes")
	force := flag.Bool("force", false, "replace an existing agent env/certificate set")
	flag.Parse()

	if *master == "" || *token == "" {
		fatal(errors.New("--master and --token are required"))
	}
	if !*force {
		for _, path := range []string{*envFile, filepath.Join(*certsDir, "agent.key")} {
			if _, err := os.Stat(path); err == nil {
				fatal(fmt.Errorf("%s already exists (use --force to replace it)", path))
			}
		}
	}

	logx.Info(logx.EnrollStart, "enrolling with master %s...", *master)
	key, err := rsa.GenerateKey(rand.Reader, 3072)
	if err != nil {
		fatal(fmt.Errorf("generate private key: %w", err))
	}
	hostname, _ := os.Hostname()
	csrDER, err := x509.CreateCertificateRequest(rand.Reader, &x509.CertificateRequest{
		Subject: pkix.Name{Organization: []string{"honey"}, CommonName: hostname},
	}, key)
	if err != nil {
		fatal(fmt.Errorf("create CSR: %w", err))
	}
	csrPEM := pem.EncodeToMemory(&pem.Block{Type: "CERTIFICATE REQUEST", Bytes: csrDER})
	logx.Info(logx.EnrollKeygen, "made a fresh key + csr, private key stays here")
	body, _ := json.Marshal(map[string]string{"csr_pem": string(csrPEM)})
	endpoint := strings.TrimRight(*master, "/") + "/enroll/" + url.PathEscape(*token) + "/claim"
	req, err := http.NewRequest(http.MethodPost, endpoint, bytes.NewReader(body))
	if err != nil {
		fatal(err)
	}
	req.Header.Set("Content-Type", "application/json")
	req.Header.Set("Accept", "application/json")
	client, err := httpClient(*masterCA)
	if err != nil {
		fatal(err)
	}
	resp, err := client.Do(req)
	if err != nil {
		fatal(fmt.Errorf("claim enrollment: %w", err))
	}
	defer resp.Body.Close()
	responseBody, err := io.ReadAll(io.LimitReader(resp.Body, 1<<20))
	if err != nil {
		fatal(err)
	}
	if resp.StatusCode != http.StatusOK {
		logx.Fatal(logx.EnrollRejected, "master rejected enrollment (%s): %s", resp.Status, strings.TrimSpace(string(responseBody)))
	}
	var claim claimResponse
	if err := json.Unmarshal(responseBody, &claim); err != nil {
		logx.Fatal(logx.EnrollBadResponse, "can't decode master response: %v", err)
	}
	if claim.NodeID == "" || claim.CertificatePEM == "" || claim.CAPEM == "" {
		logx.Fatal(logx.EnrollBadResponse, "master response is incomplete, bailing")
	}

	keyPEM := pem.EncodeToMemory(&pem.Block{Type: "RSA PRIVATE KEY", Bytes: x509.MarshalPKCS1PrivateKey(key)})
	if err := os.MkdirAll(*certsDir, 0o750); err != nil {
		fatal(err)
	}
	if err := writeAtomic(filepath.Join(*certsDir, "agent.key"), keyPEM, 0o600); err != nil {
		fatal(err)
	}
	if err := writeAtomic(filepath.Join(*certsDir, "agent.crt"), []byte(claim.CertificatePEM), 0o644); err != nil {
		fatal(err)
	}
	if err := writeAtomic(filepath.Join(*certsDir, "ca.crt"), []byte(claim.CAPEM), 0o644); err != nil {
		fatal(err)
	}
	mode := claim.Transport
	if mode == "" {
		mode = "serve"
	}
	env := fmt.Sprintf("HONEY_MODE=%s\nHONEY_LISTEN=%s\nHONEY_MASTER_ADDR=%s\nHONEY_NODE_ID=%s\nHONEY_SINGBOX_BIN=/usr/local/bin/sing-box\nHONEY_XRAY_BIN=/usr/local/bin/xray\nHONEY_CLASH_URL=http://127.0.0.1:9090\nHONEY_CLASH_SECRET=\n", mode, *listen, *masterAddr, claim.NodeID)
	if err := writeAtomic(*envFile, []byte(env), 0o600); err != nil {
		fatal(err)
	}
	logx.Info(logx.EnrollDone, "enrolled node %s, certs written", claim.NodeName)
	fmt.Printf("enrolled node %s (%s)\n", claim.NodeName, claim.NodeID)
	fmt.Printf("certificate %s expires %s\n", claim.SerialNumber, claim.ExpiresAt)
	fmt.Println("review /etc/honey/agent.env, then: systemctl enable --now honey-agent")
}

func httpClient(caFile string) (*http.Client, error) {
	transport := http.DefaultTransport.(*http.Transport).Clone()
	if caFile != "" {
		roots, err := x509.SystemCertPool()
		if err != nil || roots == nil {
			roots = x509.NewCertPool()
		}
		data, err := os.ReadFile(caFile)
		if err != nil {
			return nil, fmt.Errorf("read master CA: %w", err)
		}
		if !roots.AppendCertsFromPEM(data) {
			return nil, errors.New("master CA contains no certificates")
		}
		transport.TLSClientConfig = &tls.Config{RootCAs: roots, MinVersion: tls.VersionTLS12}
	}
	return &http.Client{Transport: transport, Timeout: 30 * time.Second}, nil
}

func writeAtomic(path string, data []byte, mode os.FileMode) error {
	if err := os.MkdirAll(filepath.Dir(path), 0o750); err != nil {
		return err
	}
	tmp, err := os.CreateTemp(filepath.Dir(path), ".honey-enroll-*")
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
	if err := os.Remove(path); err != nil && !os.IsNotExist(err) {
		return err
	}
	return os.Rename(name, path)
}

func fatal(err error) {
	logx.Fatal(logx.EnrollFatal, "enroll failed: %v", err)
}
