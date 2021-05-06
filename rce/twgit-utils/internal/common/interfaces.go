package common

import "io"

type (
	UnmarshalAt interface {
		UnmarshalAt(key string, obj interface{}) error
	}

	StringRewriter func(s string) string
	StringFilterFn func(s string) bool

	UpdateStatus string

	Updatable interface {
		Update() (UpdateStatus, error)
	}

	ReadAllable interface {
		ReadAll() ([]byte, error)
	}

	BlobConfig interface {
		Updatable
		ReadAllable
	}

	Marshaller interface {
		Marshal(o map[string]interface{}) ([]byte, error)
	}

	Unmarshaller interface {
		Unmarshal(b []byte) (map[string]interface{}, error)
	}

	// compatible with koanf's Parser() interface
	Parser interface {
		Marshaller
		Unmarshaller
	}

	KeyValueVisitorCb func(k, v string) error

	// A function that takes a KeyValueVisitorCb function and
	// calls it with each k,v pair, returning error if `f` returns
	// an error and nil if it doesn't.
	KeyValueVisitor func(f func(k, v string) error) error

	KeyValueWrapper func(visitor KeyValueVisitor, f func (k, v string) (kk, vv string)) KeyValueVisitor

	LogConfig interface {
		IsDebug() bool
		IsTrace() bool
		Output() io.Writer
	}
)

func (u UpdateStatus) String() string { return string(u) }
