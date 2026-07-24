// Package core holds what sing-box and xray share: the manager interface, the
// process lifecycle, the runtime stat shape and the protocol-agnostic
// desired-state model that each core turns into its own config.
package core

import (
	"context"
	"encoding/json"
	"time"
)

type State int

const (
	StateStopped State = iota
	StateRunning
	StateErrored
)

// UserTraffic is cumulative per-user traffic since a stats stream started.
type UserTraffic struct {
	Name string
	Up   uint64
	Down uint64
}

// Stat is one runtime sample from a core.
type Stat struct {
	NodeUp      uint64
	NodeDown    uint64
	UpSpeed     uint64
	DownSpeed   uint64
	Connections uint32
	Users       []UserTraffic
}

// LiveConn is one active connection, for the master's live-connections view.
type LiveConn struct {
	ID          string
	User        string
	SourceIP    string
	Destination string
	Network     string
	Chain       string
	Up          uint64
	Down        uint64
	StartedAtMS int64
}

// ConnLister is optionally implemented by cores that can enumerate active
// connections (sing-box, via the Clash API). Cores without it are skipped.
type ConnLister interface {
	Connections(ctx context.Context) ([]LiveConn, error)
}

// ConnCloser is optionally implemented by cores that can close active
// connections by id (sing-box, via the Clash API). Used to enforce device
// limits. Returns how many closed successfully.
type ConnCloser interface {
	CloseConnections(ctx context.Context, ids []string) (uint32, error)
}

// Manager is one managed core (sing-box or xray) on the node.
type Manager interface {
	// BuildConfig turns the (already core-filtered) spec into a config string.
	BuildConfig(spec Spec) (string, error)
	// Validate checks a candidate without changing the running process.
	Validate(configJSON string) error
	Start(configJSON string) error
	Stop() error
	Apply(configJSON string) error
	Status() (State, int, string)
	Version(ctx context.Context) (string, error)
	StatsLoop(ctx context.Context, interval time.Duration, fn func(Stat) error) error
}

// --- desired-state model (shared by both core config builders) -------------

type Spec struct {
	LogLevel    string
	ClashListen string
	ClashSecret string
	Inbounds    []Inbound
	Wireguard   []WgInterface
	Services    []NodeService
}

// NodeService is a managed external daemon (mtproto/naive) the agent runs
// directly — a separate data-plane from sing-box/xray.
type NodeService struct {
	Kind       string
	Name       string
	ListenPort uint32
	Secret     string
	ConfigJSON string
}

// WgInterface is one WireGuard / AmneziaWG server the agent runs directly (not
// through sing-box/xray). Address carries the pool prefix, e.g. "10.7.0.1/24".
type WgInterface struct {
	Name              string
	ListenPort        uint32
	PrivateKey        string
	Address           string
	MTU               uint32
	Amnezia           bool
	AmneziaParamsJSON string
	Peers             []WgPeer
}

type WgPeer struct {
	PublicKey string
	AllowedIP string // client /32
}

type Inbound struct {
	Core      string // "singbox" | "xray"
	Tag       string
	Type      string
	Listen    string
	Port      uint32
	Users     []User
	TLS       *TLS
	Transport *Transport
	ExtraJSON json.RawMessage
	UpMbps    uint32 // bandwidth cap (Mbps), 0 = unlimited; hysteria2
	DownMbps  uint32
	// multihop: a full sing-box outbound (JSON) to the exit inbound. When set,
	// the agent adds it and routes this inbound's traffic through it.
	UpstreamOutboundJSON string
}

type User struct {
	Name       string
	UUID       string
	Password   string
	Flow       string
	QuotaBytes uint64 // remaining quota for local cutoff; 0 = unlimited
}

// Transport is the network layer under the protocol.
type Transport struct {
	Network     string // tcp | ws | grpc | http | httpupgrade | xhttp | quic | mkcp
	Path        string
	Host        string
	ServiceName string
	Mode        string // xhttp
}

type TLS struct {
	Enabled         bool
	ServerName      string
	CertPath        string
	KeyPath         string
	Reality         *Reality
	ECH             bool
	UTLSFingerprint string
	// shadowtls masquerade target (type == "shadowtls")
	ShadowTLSHandshakeServer string
	ShadowTLSHandshakePort   uint32
}

type Reality struct {
	PrivateKey      string
	ShortIDs        []string
	HandshakeServer string
	HandshakePort   uint32
}
