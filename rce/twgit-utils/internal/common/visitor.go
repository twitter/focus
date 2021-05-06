package common

import (
	"math"
	"os"
	"strings"
)

// EnvMap returns a map of the environment os.Environ returns
func EnvMap() (m map[string]string) {
	m = make(map[string]string)

	// i can't imagine why the stdlib doesn't provide this i mean really guys
	for _, kv := range os.Environ() {
		i := strings.Index(kv, "=")
		m[kv[:i]] = kv[i+1:]
	}

	return m
}

func VisitEnv(pairs []string, f func(k, v string) error) (err error) {
	for _, kv := range pairs {
		i := strings.Index(kv, "=")
		k := kv[:i]
		v := kv[i+1:]
		if err = f(k, v); err != nil {
			return err
		}
	}
	return nil
}

func NewEnvVisitor(pairs []string) KeyValueVisitor {
	return func(cb func(k, v string) error) error {
		return VisitEnv(pairs, cb)
	}
}

//var EnvVisitor KeyValueVisitor = NewEnvVisitor(nil)

func NewMapVisitor(m map[string]string) KeyValueVisitor {
	return func(cb func(k, v string) error) (err error) {
		for k, v := range m {
			if err = cb(k, v); err != nil {
				return err
			}
		}
		return nil
	}
}

// NewPairsVisitor returns a KeyValueVisitor that will iterate over pairs of
// strings in xs. evens are keys odds are values. This visitor preserves the
// order in xs, whereas the MapVisitor does not.
// Panics if len(xs) is not an even number
//
func NewPairsVisitor(xs ...string) KeyValueVisitor {
	if int(math.Mod(float64(len(xs)), 2.0)) != 0 {
		panic("NewPairsVisitor was given an odd number of arguments")
	}

	return func(cb func(k, v string) error) (err error) {
		for i := 0; i < len(xs); i += 2 {
			if err = cb(xs[i], xs[i+1]); err != nil {
				return err
			}
		}
		return nil
	}
}
