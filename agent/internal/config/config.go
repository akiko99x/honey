// Package config holds the agent's runtime settings, parsed from flags.
// (env/file loading can layer on top later.)
package config

import "flag"

type Config struct {
	Mode       string // transport mode: "serve" | "dial" | "both"
	Listen     string // grpc listen addr for serve mode, e.g. 0.0.0.0:8443
	MasterAddr string // master tunnel acceptor for dial mode, e.g. 203.0.113.10:9443

	CAFile   string
	CertFile string
	KeyFile  string

	NodeID string

	SingboxBin    string // path to sing-box binary
	SingboxConfig string // where the agent writes config.json

	ClashURL    string // sing-box Clash API base, e.g. http://127.0.0.1:9090
	ClashSecret string // Clash API secret, if set

	XrayBin        string // path to xray binary
	XrayConfig     string // where the agent writes xray config.json
	XrayAPI        string // xray gRPC stats api addr (stats collection TBD)
	HysteriaBin    string // path to official Hysteria binary
	HysteriaConfig string // where the agent writes Hysteria server config

	XrayACMERoot        string // persistent Xray ACME cache + exported PEMs
	XrayACMEListen      string // local HTTP-01 gateway
	SingboxACMEUpstream string // sing-box HTTP-01 listener behind the gateway
}

// Parse reads flags into a Config.
func Parse() *Config {
	c := &Config{}
	flag.StringVar(&c.Mode, "mode", "serve", "transport mode: serve | dial | both")
	flag.StringVar(&c.Listen, "listen", "0.0.0.0:8443", "grpc listen address (serve/both)")
	flag.StringVar(&c.MasterAddr, "master-addr", "", "master tunnel acceptor host:port (dial/both)")
	flag.StringVar(&c.CAFile, "ca", "/etc/honey/certs/ca.crt", "ca cert path")
	flag.StringVar(&c.CertFile, "cert", "/etc/honey/certs/agent.crt", "agent cert path")
	flag.StringVar(&c.KeyFile, "key", "/etc/honey/certs/agent.key", "agent key path")
	flag.StringVar(&c.NodeID, "node-id", "node-1", "stable node id")
	flag.StringVar(&c.SingboxBin, "singbox-bin", "/usr/local/bin/sing-box", "sing-box binary")
	flag.StringVar(&c.SingboxConfig, "singbox-config", "/etc/honey/sing-box/config.json", "sing-box config path")
	flag.StringVar(&c.ClashURL, "clash-url", "http://127.0.0.1:9090", "clash api base url")
	flag.StringVar(&c.ClashSecret, "clash-secret", "", "clash api secret")
	flag.StringVar(&c.XrayBin, "xray-bin", "/usr/local/bin/xray", "xray binary")
	flag.StringVar(&c.XrayConfig, "xray-config", "/etc/honey/xray/config.json", "xray config path")
	flag.StringVar(&c.XrayAPI, "xray-api", "127.0.0.1:8081", "xray grpc stats api addr")
	flag.StringVar(&c.HysteriaBin, "hysteria-bin", "/usr/local/bin/hysteria", "official Hysteria binary")
	flag.StringVar(&c.HysteriaConfig, "hysteria-config", "/etc/honey/hysteria/config.json", "Hysteria server config path")
	flag.StringVar(&c.XrayACMERoot, "xray-acme-root", "/etc/honey/xray/acme", "Xray ACME state directory")
	flag.StringVar(&c.XrayACMEListen, "xray-acme-listen", "127.0.0.1:9080", "Xray ACME HTTP-01 gateway")
	flag.StringVar(&c.SingboxACMEUpstream, "singbox-acme-upstream", "127.0.0.1:9082", "sing-box ACME listener behind the gateway")
	flag.Parse()
	return c
}
