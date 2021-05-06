package userhash

import (
	"github.com/imdario/mergo"
	"github.com/pkg/errors"

	log "github.com/sirupsen/logrus"

	con "github.com/buraksezer/consistent"
	"github.com/cespare/xxhash"
)

type (

	// we define our own version of this struct so we can
	// tag the fields and (un)marshal it
	HasherConfig struct {
		PartitionCount    int
		ReplicationFactor int
		Load              float64
		Nodes             []string
	}

	HashRing struct {
		ring *con.Consistent
	}

	Hasher interface {
		Locate(string) string
	}

	xxh struct{}

	member string
)

func (m member) String() string {
	return string(m)
}

func (h xxh) Sum64(data []byte) uint64 {
	return xxhash.Sum64(data)
}

func (h *HashRing) Locate(s string) string {
	return h.ring.LocateKey([]byte(s)).String()
}

func (hc HasherConfig) Hasher() (hr Hasher, err error) {
	return New(hc)
}

// Merge the values from o onto the receiver. this mutates the receiver
// when a field is using the default values
func (hc *HasherConfig) Merge(o HasherConfig) {
	if err := mergo.Merge(hc, o); err != nil {
		log.Fatalf("failed to merge %#v and %#v, error: %#v", hc, o, err)
	}
}

func New(hc HasherConfig) (hr Hasher, err error) {
	if hc.Nodes == nil {
		return nil, errors.New("Dev server list was empty")
	}

	if hc.PartitionCount < len(hc.Nodes) {
		return nil, errors.Errorf(
			"PartitionCount < num dev hosts (%d < %d)",
			hc.PartitionCount,
			len(hc.Nodes),
		)
	}

	cfg := con.Config{
		Hasher:            xxh{},
		PartitionCount:    hc.PartitionCount,
		ReplicationFactor: hc.ReplicationFactor,
		Load:              hc.Load,
	}

	log.WithFields(log.Fields{
		"PartitionCount":    hc.PartitionCount,
		"ReplicationFactor": hc.ReplicationFactor,
		"Load":              hc.Load,
		"NumNodes":          len(hc.Nodes),
	}).Debug("hasher config")

	var nodes []con.Member
	for _, node := range hc.Nodes {
		nodes = append(nodes, member(node))
	}

	return &HashRing{con.New(nodes, cfg)}, nil
}
