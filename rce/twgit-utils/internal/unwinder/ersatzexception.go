package unwinder

import "github.com/pkg/errors"

type (
	// U wraps panic/recover and provides a CheckErr and Errorf
	U struct {
		ce chan error
	}

	WrappedErr error
)

func (b *U) Check(err error) {
	if err != nil {
		panic(WrappedErr(err))
	}
}

// Errorf forces an unwind of the stack with an error created
// with format string 'str' and args passed to github.com/pkg/errors.Errorf
func (b *U) Errorf(str string, args ...interface{}) {
	panic(WrappedErr(errors.Errorf(str, args...)))
}

func (b *U) recover() {
	if err := recover(); err != nil {
		if e, ok := err.(WrappedErr); ok {
			b.ce <- e
		} else {
			panic(err)
		}
	}
}

func (b *U) close() { close(b.ce) }

func Run(f func(u *U)) (err error) {
	b := &U{make(chan error)}
	go func() {
		defer b.close()
		defer b.recover()
		f(b)
	}()

	for err := range b.ce {
		if err != nil {
			return err
		}
	}
	return nil
}
