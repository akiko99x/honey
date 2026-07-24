package singbox

import (
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"time"
)

// Clash is a minimal client for sing-box's experimental Clash API.
type Clash struct {
	base   string
	secret string
	http   *http.Client
}

func NewClash(base, secret string) *Clash {
	return &Clash{
		base:   base,
		secret: secret,
		http:   &http.Client{Timeout: 5 * time.Second},
	}
}

// Conn is one active connection as seen by the Clash API.
type Conn struct {
	ID          string
	User        string // inbound user (= panel username), empty if unauthenticated
	SourceIP    string
	Destination string // requested host, else dest ip:port
	Network     string // tcp | udp
	Chain       string // outbound chain (last hop), best-effort
	Up          uint64 // cumulative bytes for this connection
	Down        uint64
	StartedAtMS int64 // unix millis, 0 if unknown
}

// Snapshot is a point-in-time read of the Clash /connections endpoint.
type Snapshot struct {
	UpBytes     uint64 // node total upload
	DownBytes   uint64 // node total download
	Connections uint32
	Conns       []Conn
}

// connectionsResponse mirrors the fields we care about from GET /connections.
type connectionsResponse struct {
	DownloadTotal uint64 `json:"downloadTotal"`
	UploadTotal   uint64 `json:"uploadTotal"`
	Connections   []struct {
		ID       string   `json:"id"`
		Upload   uint64   `json:"upload"`
		Download uint64   `json:"download"`
		Start    string   `json:"start"` // RFC3339 connection start
		Chains   []string `json:"chains"`
		Metadata struct {
			User            string `json:"user"`
			Network         string `json:"network"`
			SourceIP        string `json:"sourceIP"`
			DestinationIP   string `json:"destinationIP"`
			DestinationPort string `json:"destinationPort"`
			Host            string `json:"host"`
		} `json:"metadata"`
	} `json:"connections"`
}

// Read pulls a fresh snapshot from the Clash API.
func (c *Clash) Read(ctx context.Context) (Snapshot, error) {
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, c.base+"/connections", nil)
	if err != nil {
		return Snapshot{}, err
	}
	if c.secret != "" {
		req.Header.Set("Authorization", "Bearer "+c.secret)
	}

	resp, err := c.http.Do(req)
	if err != nil {
		return Snapshot{}, fmt.Errorf("clash /connections: %w", err)
	}
	defer resp.Body.Close()

	if resp.StatusCode != http.StatusOK {
		return Snapshot{}, fmt.Errorf("clash /connections: status %d", resp.StatusCode)
	}

	var body connectionsResponse
	if err := json.NewDecoder(resp.Body).Decode(&body); err != nil {
		return Snapshot{}, fmt.Errorf("decode clash response: %w", err)
	}

	conns := make([]Conn, 0, len(body.Connections))
	for _, c := range body.Connections {
		dest := c.Metadata.Host
		if dest == "" {
			dest = c.Metadata.DestinationIP
			if c.Metadata.DestinationPort != "" {
				dest = dest + ":" + c.Metadata.DestinationPort
			}
		}
		var startedMS int64
		if c.Start != "" {
			if t, err := time.Parse(time.RFC3339, c.Start); err == nil {
				startedMS = t.UnixMilli()
			}
		}
		chain := ""
		if len(c.Chains) > 0 {
			chain = c.Chains[0] // outermost outbound
		}
		conns = append(conns, Conn{
			ID:          c.ID,
			User:        c.Metadata.User,
			SourceIP:    c.Metadata.SourceIP,
			Destination: dest,
			Network:     c.Metadata.Network,
			Chain:       chain,
			Up:          c.Upload,
			Down:        c.Download,
			StartedAtMS: startedMS,
		})
	}

	return Snapshot{
		UpBytes:     body.UploadTotal,
		DownBytes:   body.DownloadTotal,
		Connections: uint32(len(body.Connections)),
		Conns:       conns,
	}, nil
}

// Close asks the Clash API to close one active connection by id.
func (c *Clash) Close(ctx context.Context, id string) error {
	req, err := http.NewRequestWithContext(ctx, http.MethodDelete, c.base+"/connections/"+id, nil)
	if err != nil {
		return err
	}
	if c.secret != "" {
		req.Header.Set("Authorization", "Bearer "+c.secret)
	}
	resp, err := c.http.Do(req)
	if err != nil {
		return fmt.Errorf("clash close %s: %w", id, err)
	}
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusOK && resp.StatusCode != http.StatusNoContent {
		return fmt.Errorf("clash close %s: status %d", id, resp.StatusCode)
	}
	return nil
}
