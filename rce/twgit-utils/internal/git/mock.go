package git

type (
	mockCommandResult struct {
		output   []string
		errors   []string
		exitCode int
	}

	ArgCallback func(args ...string) error
)

var NoOpArgCallback ArgCallback = func(args ...string) error { return nil }

func (r *mockCommandResult) OutputLines() []string { return r.output }
func (r *mockCommandResult) ErrorLines() []string  { return r.errors }
func (r *mockCommandResult) ExitCode() int         { return r.exitCode }
func (r *mockCommandResult) Command() *GitCmd      { panic("not implemented") }

func NewMockCommandRunner(output []string, errors []string, exitCode int, cb ArgCallback) CommandRunner {
	return func(args ...string) (cr CommandResult, err error) {
		if cb != nil {
			if err = cb(args...); err != nil {
				return nil, err
			}
		}
		return &mockCommandResult{output, errors, exitCode}, nil
	}
}
