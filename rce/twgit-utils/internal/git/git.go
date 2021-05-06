package git

import (
	"bufio"
	"bytes"
	"fmt"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"strings"

	"git.twitter.biz/focus/rce/twgit-utils/internal/common"
	"github.com/pkg/errors"
	log "github.com/sirupsen/logrus"
)

func GitBin() string {
	var err error
	var gitBin string
	if gitBin, err = exec.LookPath("git"); err != nil {
		log.Fatal(errors.Wrap(err, "could not locate git binary"))
	}

	return gitBin
}

type (
	NotAGitRepo struct {
		Path string
	}

	GitCmd struct {
		*exec.Cmd
		Stdout bytes.Buffer
		Stderr bytes.Buffer
	}

	// A function that memoizes a Repo on first use and
	// returns the same instance each time its called.
	LazyGit func() *Repo
)

// NewMustLazyGit returns a memoizing function that returns a *Repo.
// It panics if it fails to create the Repo on the first call.
func NewMustLazyGit(repoPath string) LazyGit {
	var repo *Repo
	return func() *Repo {
		if repo == nil {
			var err error
			repo, err = NewRepo(repoPath)
			if err != nil {
				panic(err)
			}
		}
		return repo
	}
}

// ConstLazyGit wraps an existing git repo in a function so it satisfies the
// LazyGit interface
func ConstLazyGit(repo *Repo) LazyGit {
	return func() *Repo { return repo }
}

func NewGitCmd(repoPath string) (cmd *GitCmd, err error) {
	abs, err := filepath.Abs(repoPath)

	if err != nil {
		return nil, errors.Wrapf(err, "failed to convert repoPath %#v to an absolute path", repoPath)
	}

	cmd = &GitCmd{
		Cmd: exec.Command(GitBin(), "-C", abs),
	}

	cmd.Cmd.Env = os.Environ()
	cmd.Cmd.Stdout = &cmd.Stdout
	cmd.Cmd.Stderr = io.MultiWriter(&cmd.Stderr, os.Stderr)

	cmd.SetEnv("LC_ALL", "C")

	return cmd, nil
}

func (g *GitCmd) AddArgs(args ...string) *GitCmd {
	g.Cmd.Args = append(g.Cmd.Args, args...)
	return g
}

func (g *GitCmd) AddArgf(f string, opts ...interface{}) *GitCmd {
	return g.AddArgs(fmt.Sprintf(f, opts...))
}

func (g *GitCmd) SetEnv(k, v string) *GitCmd {
	g.Env = append(g.Env, fmt.Sprintf("%s=%s", k, v))
	return g
}

// CommandLine returns the comand executed as a string for debugging
func (g *GitCmd) CommandLine() string {
	return strings.Join(g.Args, " ")
}

func (g *GitCmd) Run() (err error) {
	// do some common error checking in here

	err = g.Cmd.Run()

	log.WithFields(log.Fields{
		"cmd":      g.Cmd.String(),
		"exitCode": g.Cmd.ProcessState.ExitCode(),
		"exited":   g.Cmd.ProcessState.Exited(),
		"string?":  g.Cmd.ProcessState.String(),
		"err":      err,
	}).Debug()

	if err != nil {
		emsg := g.Stderr.String()
		if strings.Contains(emsg, "fatal: not a git repository") {
			return errors.Wrapf(&NotAGitRepo{Path: g.Path}, "git command failed - not a git repo")
		}
	}

	return g.Check()
}

func (g *GitCmd) Check() (err error) {
	if g.ProcessState == nil {
		return nil
	}
	if !g.ProcessState.Success() {
		return &CommandFailedError{
			Command:  g.String(),
			ExitCode: g.ProcessState.ExitCode(),
			Stderr:   g.Stderr.String(),
		}
	}
	return nil
}

func (g *GitCmd) Output() (out []byte, err error) {
	if err = g.Run(); err != nil {
		return nil, err
	}
	return g.Stdout.Bytes(), nil
}

func scanLines(r io.Reader) (lines []string) {
	scan := bufio.NewScanner(r)
	for scan.Scan() {
		lines = append(lines, scan.Text())
	}
	return lines
}

/* CommandResult interface */

// Output returns the lines of output from the command as a slice of strings
// with trailing newlines removed
func (g *GitCmd) OutputLines() (lines []string) { return scanLines(&g.Stdout) }
func (g *GitCmd) ErrorLines() (lines []string)  { return scanLines(&g.Stderr) }
func (g *GitCmd) ExitCode() int                 { return g.ProcessState.ExitCode() }
func (g *GitCmd) Command() *GitCmd              { return g }

/* end */

var _ error = &NotAGitRepo{}

func (e *NotAGitRepo) Error() string {
	return fmt.Sprintf("%s is not a git repository", e.Path)
}

func IsBareRepository(path string) (ok bool, err error) {
	cmd := exec.Command("git", "-C", path, "rev-parse", "--is-bare-repository")

	var stdout bytes.Buffer
	var stderr bytes.Buffer

	cmd.Env = append(os.Environ(), "LC_ALL=C")
	cmd.Stdout = &stdout
	cmd.Stderr = io.MultiWriter(&stderr, os.Stderr)

	if err = cmd.Run(); err != nil {
		return false, err
	}

	log.WithFields(log.Fields{
		"cmd":      cmd.String(),
		"exitCode": cmd.ProcessState.ExitCode(),
		"exited":   cmd.ProcessState.Exited(),
		"string?":  cmd.ProcessState.String(),
	}).Debug()

	ok = stdout.String() == "true\n"

	return ok, nil
}

type (
	Repo struct {
		path     string
		gitDir   string
		isBare   bool
		extraEnv []string
	}

	CommandResult interface {
		OutputLines() []string
		ErrorLines() []string
		ExitCode() int
		Command() *GitCmd
	}

	CommandRunner func(cmd ...string) (CommandResult, error)
)

var _ CommandResult = new(GitCmd)
var _ CommandRunner = new(Repo).Run

func NewRepo(path string) (repo *Repo, err error) {
	if path == "" {
		path = "."
	}

	repo = &Repo{path: filepath.Clean(path)}

	if repo.isBare, err = IsBareRepository(path); err != nil {
		return nil, err
	}

	cmd, err := NewGitCmd(path)
	if err != nil {
		return nil, err
	}
	cmd.AddArgs("rev-parse", "--git-dir")
	if err = cmd.Run(); err != nil {
		return nil, err
	}

	repo.gitDir = filepath.Clean(
		filepath.Join(path, strings.TrimSpace(cmd.Stdout.String())))

	return repo, nil
}

// AddExtraEnv adds the given strings (in os.Environ style "FOO=bar") to the execution
// of commands against this repo. This allows the user to set 'GIT_*' env vars for
// controlling its behavior
func (r *Repo) AddExtraEnv(env ...string) {
	r.extraEnv = append(r.extraEnv, env...)
}

// Run takes a list of arguments (not including 'git') to run in this repo
// executes the command, and returns the GitCmd for further inspection.
// This is not fancy and if you need more sophisticated control, use Cmd().
// We wlll return the GitCmd struct whether or not err is nil, so that the
// caller may inspect Stderr for clues.
func (r *Repo) Run(args ...string) (cr CommandResult, err error) {
	return r.run(args...)
}

func (r *Repo) run(args ...string) (cr *GitCmd, err error) {
	if len(args) > 1 && args[0] == "git" {
		return nil, errors.Errorf(
			"invalid command, argument to Run's first value should not be \"git\". "+
				"args: %#v",
			args,
		)
	}

	cmd, err := r.Cmd()
	if err != nil {
		return nil, err
	}
	cmd.AddArgs(args...)
	err = cmd.Run()
	return cmd, err
}

// Cmd returns a GitCmd struct set up to execute git subcommands in this
// repository.
func (r *Repo) Cmd() (cmd *GitCmd, err error) {
	if cmd, err = NewGitCmd(r.path); err != nil {
		return nil, err
	}

	cmd.Env = append(cmd.Env, r.extraEnv...)
	return cmd, nil
}

func (r *Repo) Path() string   { return r.path }
func (r *Repo) GitDir() string { return r.gitDir }
func (r *Repo) IsBare() bool   { return r.isBare }

// return a path relative to the top level of this repository. This method will panic
// if IsBare returns true.
func (r *Repo) RelPath(ps ...string) string {
	if r.isBare {
		panic("Tried to call RelPath on a bare repository: " + r.path)
	}
	return filepath.Join(append([]string{r.path}, ps...)...)
}

type CommandFailedError struct {
	Command  string
	ExitCode int
	Stderr   string
}

func (e *CommandFailedError) Error() string {
	return fmt.Sprintf(
		"the command %#v exited with exitstatus %#v.stderr was: %#v",
		e.Command, e.ExitCode, e.Stderr,
	)
}

// CatFileBlob returns the contents of the repository relative path at the given
// ref. The ref does not need to be a literal reference, rather it needs to
// "spell a commit". For more info see gitrevisions(7).
func (f *Repo) CatFileBlob(ref, path string) (data []byte, err error) {
	cmd, err := f.Cmd()
	if err != nil {
		return nil, err
	}

	cmd.AddArgs("cat-file", "blob").AddArgf("%s:%s", ref, path)
	if err = cmd.Run(); err != nil {
		return nil, err
	}

	return cmd.Stdout.Bytes(), nil
}

type NoWorkingTreeError struct {
	Operation string
	Path      string
}

func (e *NoWorkingTreeError) Error() string {
	return fmt.Sprintf(
		"this operation (%#v) must be run in a work tree, path: %#v",
		e.Operation,
		e.Path,
	)
}

// TopLevel returns the path to the top level of the repository as an abspath.
// If this is a bare repo, a NoWorkingTreeError will be returned.
func (r *Repo) TopLevel() (path string, err error) {
	var cmd *GitCmd

	if r.isBare {
		return "", &NoWorkingTreeError{"git rev-parse --show-toplevel", r.path}
	}

	if cmd, err = r.run("rev-parse", "--show-toplevel"); err != nil {
		return "", err
	}

	out := cmd.OutputLines()
	if len(out) < 1 {
		return "", errors.Errorf("rev-parse --show-toplevel returned no output")
	}

	return out[0], nil
}

type (
	Config struct {
		repo  *Repo
		scope string
		args  []string
	}
)

var _ common.KeyValueVisitor = new(Config).Visit

func (r *Repo) Config() *Config {
	return &Config{r, "", nil}
}

func copyArgs(s []string) []string {
	ns := make([]string, len(s), cap(s))
	copy(ns, s)
	return ns
}

// Local returns a Config instance with '--local' scope set
func (c *Config) Local() *Config { return &Config{c.repo, "--local", copyArgs(c.args)} }

// Global returns a Config with the '--global' scope set
func (c *Config) Global() *Config { return &Config{c.repo, "--global", copyArgs(c.args)} }

func (c *Config) mkArgs() (args []string) {
	args = append(args, "config")
	if c.scope != "" {
		args = append(args, c.scope)
	}
	return append(args, c.args...)
}

func (c *Config) Type(s string) *Config {
	newc := &Config{c.repo, c.scope, copyArgs(c.args)}
	switch s {
	case "bool", "int", "bool-or-int", "path", "expiry-date", "color":
		newc.args = append(newc.args, "--type="+s)
	default:
		log.Panicf("invalid type given as argument: %#v", s)
	}
	return newc
}

func (c *Config) allLines() (lines []string, err error) {
	cmd, err := c.repo.Cmd()
	if err != nil {
		return nil, err
	}
	cmd.AddArgs(c.mkArgs()...).AddArgs("--list")

	// note: err is nil even if ExitCode > 0
	if err = cmd.Run(); err != nil {
		return nil, err
	}

	if code := cmd.ExitCode(); code > 0 {
		return nil, errors.Errorf("git command %#v failed with exit code %#v", cmd.CommandLine(), code)
	}

	lines = cmd.OutputLines()
	for i := range lines {
		lines[i] = strings.TrimSpace(lines[i])
	}

	return lines, nil
}

func (c *Config) Visit(f func(k, v string) error) (err error) {
	lines, err := c.allLines()
	if err != nil {
		return err
	}

	for i := range lines {
		q := strings.Index(lines[i], "=")
		if q < 0 {
			log.Warnf("malformed line from git status; %#v", lines[i])
		}

		k := lines[i][:q]
		v := lines[i][q+1:]
		if err = f(k, v); err != nil {
			return err
		}
	}

	return nil
}

// Get returns the value for 'key' for the currently defined git config scope.
// For a missing key, this function returns val="", ok=false, error=nil.
func (c *Config) Get(key string) (val string, ok bool, err error) {
	cmd, err := c.repo.Cmd()
	if err != nil {
		return "", false, err
	}
	cmd.AddArgs(c.mkArgs()...).AddArgs("--get", key)

	if err = cmd.Run(); err != nil {
		if cfe, ok := err.(*CommandFailedError); ok {
			// this is what git does when we try to get a missing key
			if cfe.ExitCode == 1 && cfe.Stderr == "" {
				return "", false, nil
			}
		}

		return "", false, err
	}

	lines := cmd.OutputLines()
	if len(lines) < 1 {
		log.Fatalf("no output from %#v when 1 line was expected", cmd.String())
	}

	return strings.TrimSpace(lines[0]), true, nil
}

func (c *Config) Set(key, val string) (err error) {
	cmd, err := c.repo.Cmd()
	if err != nil {
		return err
	}

	cmd.AddArgs(c.mkArgs()...).AddArgs(key, val)

	// Run calls Check so this will do the right thing
	return cmd.Run()
}
