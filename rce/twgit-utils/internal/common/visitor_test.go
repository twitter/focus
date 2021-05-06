package common

import (
	"testing"

	r "github.com/stretchr/testify/require"
)

type (
	Pair struct {
		K string
		V string
	}
)

func TestNewWrapper(t *testing.T) {
	f := r.New(t)

	pv := NewPairsVisitor("a", "b", "c", "d", "e", "f")

	wrapped := NewWrapper(pv, func(k, v string) (kk string, vv string, ok bool) {
		switch k {
		case "a", "e":
			return k + k, v + v, true
		case "c":
			return "", "", false
		default:
			panic("what the hell, man?!")
		}
	})

	var result []Pair
	wrapped(func(k, v string) error {
		result = append(result, Pair{k, v})
		return nil
	})

	f.NotEmpty(result)
	f.Equal(
		[]Pair{
			{"aa", "bb"},
			{"ee", "ff"},
		},
		result,
	)
}
