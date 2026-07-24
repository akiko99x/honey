// Package mtls builds the server-side TLS config the agent uses for gRPC.
// The agent presents its own cert and *requires* a valid client cert (the master),
// both signed by the honey CA.
package mtls

import (
	"crypto/tls"
	"crypto/x509"
	"fmt"
	"os"
)

// ServerConfig loads the agent cert/key and the CA used to verify the master.
func ServerConfig(caPath, certPath, keyPath string) (*tls.Config, error) {
	cert, err := tls.LoadX509KeyPair(certPath, keyPath)
	if err != nil {
		return nil, fmt.Errorf("load agent keypair: %w", err)
	}

	caPEM, err := os.ReadFile(caPath)
	if err != nil {
		return nil, fmt.Errorf("read ca: %w", err)
	}
	pool := x509.NewCertPool()
	if !pool.AppendCertsFromPEM(caPEM) {
		return nil, fmt.Errorf("ca %q: no certs parsed", caPath)
	}

	return &tls.Config{
		Certificates: []tls.Certificate{cert},
		ClientCAs:    pool,
		ClientAuth:   tls.RequireAndVerifyClientCert, // mTLS: master must present a valid cert
		MinVersion:   tls.VersionTLS13,
	}, nil
}
