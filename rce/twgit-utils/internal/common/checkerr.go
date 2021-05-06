package common

import (
	"os/exec"

	"github.com/pkg/errors"
	log "github.com/sirupsen/logrus"
)

/* this is the dumbest API decision i've seen in a long long time  */
type stackTracer interface {
	StackTrace() errors.StackTrace
}

func LogStack(err error) {
	if st, ok := err.(stackTracer); ok {
		for _, f := range st.StackTrace() {
			log.Errorf("%+s:%d\n", f, f)
		}
	}
}

func CheckErr(err error) {
	if err == nil {
		return
	}

	LogStack(err)
	log.Fatal(err)
}

func ProcessExitErr(cmd *exec.Cmd) error {
	if code := cmd.ProcessState.ExitCode(); code > 0 {
		return errors.Errorf(
			"Command %#v failed with exit code %#v",
			cmd.String(), code,
		)
	}
	return nil
}
