package testutils

import (
	"bytes"
	"net/url"
	"os"
	"os/exec"
	"runtime"
	"strings"
	"testing"

	"git.twitter.biz/focus/rce/twgit-utils/internal/common"
	"git.twitter.biz/focus/rce/twgit-utils/internal/git"
	log "github.com/sirupsen/logrus"

	fpath "path/filepath"

	r "github.com/stretchr/testify/require"
)

type (
	Fixture struct {
		*r.Assertions
		T          *testing.T
		origEnv    []string
		ConfigPath string
		Temp       string
		TestRepo   *TestRepo
		TestOrigin *TestRepo
	}

	TestRepo struct {
		*git.Repo
		r *r.Assertions
	}
)

func NewTestRepo(g *git.Repo, r *r.Assertions) *TestRepo {
	return &TestRepo{g, r}
}

// Writes the contents at the relative path given. If the contents are
// blank then just write the relative path as the contents of the file
func (t *TestRepo) WriteFile(relpath, content string) {
	fp, err := os.OpenFile(t.RelPath(relpath), os.O_WRONLY|os.O_TRUNC|os.O_CREATE, 0644)
	t.r.NoError(err)
	defer fp.Close()

	if content == "" {
		content = relpath
	}
	_, err = fp.WriteString(content)
	t.r.NoError(err)
}

func NewFixture(t *testing.T) (fix *Fixture) {
	f := &Fixture{
		Assertions: r.New(t),
		T:          t,
		origEnv:    os.Environ(),
		Temp:       t.TempDir(),
	}

	f.ResetEnv()

	f.ConfigPath = "config/twgit.yaml"
	return f
}

// put the env back exactly the way we found it
func (f *Fixture) cleanEnv() {
	os.Clearenv()
	for i := range f.origEnv {
		p := strings.Index(f.origEnv[i], "=")
		if p < 0 {
			continue
		}

		k := f.origEnv[i][0:p]
		v := f.origEnv[i][p+1:]

		os.Setenv(k, v)
	}
}

// clean the env but set a few special vars
func (f *Fixture) ResetEnv() {
	f.cleanEnv()
	os.Setenv("GIT_CONFIG_NOSYSTEM", "1")
	// setting this prevents git from finding ~/.gitconfig and messing up tests
	os.Setenv("HOME", f.Temp)
	os.Setenv("TWGIT_TEST", "true")
}

// Sets os.Environ back to what it was when NewFixture was called
func (f *Fixture) Close() {
	f.cleanEnv()
}

func (f *Fixture) GetProjectRoot() string {
	_, filename, _, _ := runtime.Caller(0)
	dir := fpath.Clean(fpath.Join(fpath.Dir(filename), "../.."))
	_, err := os.Stat(fpath.Join(dir, "go.mod"))
	log.Debugf("Project root is: %s", dir)
	f.NoError(err, "could not determine top level directory")
	return dir
}

func (f *Fixture) RootRelativeJoin(args ...string) string {
	return fpath.Join(append([]string{f.GetProjectRoot()}, args...)...)
}

// Reads a file relative to the project root (the directory that contains
// this package's go.mod file). Failure to read the file will be recorded
// on the test instance given in the constructor.
func (f *Fixture) ReadProjectRootRelativeFile(args ...string) (data []byte) {
	path := f.RootRelativeJoin(args...)
	data, err := os.ReadFile(path)
	f.NoError(err, "failed to read file: %#v", path)
	return data
}

func (f *Fixture) MustParseUrl(s string) (u *url.URL) {
	u, err := url.Parse(s)
	f.NoError(err)
	return u
}

func (f *Fixture) SetupGitTestRepos() *Fixture {
	setupRepoScript := f.RootRelativeJoin("internal", "testutils", "setup_test_repos.sh")
	var stdout bytes.Buffer
	var stderr bytes.Buffer

	testRepoPath := fpath.Join(f.Temp, "repo")
	testOriginPath := fpath.Join(f.Temp, "origin.git")

	cmd := exec.Command("bash", setupRepoScript)

	// set these values in the process environment to make git work right
	gitEnv := []string{
		"PAGER=cat",
		"EDITOR=:",
		"GIT_AUTHOR_NAME=Capt Spaulding",
		"GIT_AUTHOR_EMAIL=captspaulding@scotland-yard.co.uk",
		"GIT_COMMITTER_NAME=Roscoe W Chandler",
		"GIT_COMMITTER_EMAIL=abey@thefishman.gov",
	}

	common.NewEnvVisitor(gitEnv)(func(k, v string) error {
		return os.Setenv(k, v)
	})

	cmd.Env = os.Environ()

	cmd.Env = append(
		cmd.Env,
		"TEST_TEMPDIR="+f.Temp,
		"CONFIG_FILE="+f.RootRelativeJoin(f.ConfigPath),
		"GIT_TRACE2=1",
		"TEST_REPO="+testRepoPath,
		"TEST_ORIGIN="+testOriginPath,
	)
	cmd.Stdout = &stdout
	cmd.Stderr = &stderr

	f.NoError(cmd.Run(), "stderr: %s", stderr.String())
	f.Equal(0, cmd.ProcessState.ExitCode())
	var err error
	tr, err := git.NewRepo(testRepoPath)
	f.NoError(err)
	f.TestRepo = NewTestRepo(tr, f.Assertions)
	f.TestRepo.AddExtraEnv(gitEnv...)

	to, err := git.NewRepo(testOriginPath)
	f.NoError(err)
	f.TestOrigin = NewTestRepo(to, f.Assertions)
	f.TestOrigin.AddExtraEnv(gitEnv...)

	return f
}

func (f *Fixture) RefMustExist(r *git.Repo, name string) {
	cr, err := r.Run("show-ref", "--verify", "--quiet", "--", name)
	f.NoError(err)
	f.Equal(0, cr.ExitCode())
}
