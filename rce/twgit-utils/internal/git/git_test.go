package git_test

import (
	"bytes"
	"fmt"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"strings"

	"testing"

	"git.twitter.biz/focus/rce/twgit-utils/internal/common"
	"git.twitter.biz/focus/rce/twgit-utils/internal/git"
	"git.twitter.biz/focus/rce/twgit-utils/internal/testutils"
)

type (
	Fixture struct {
		*testutils.Fixture
	}
)

func NewFixture(t *testing.T) *Fixture {
	return &Fixture{testutils.NewFixture(t)}
}

func TestNewMustLazyGitPanicsIfArgIsNotAGitRepo(t *testing.T) {
	f := NewFixture(t)
	defer f.Close()

	f.NotEmpty(f.Temp)
	st, err := os.Stat(f.Temp)
	f.NoError(err)
	f.True(st.IsDir())

	f.Panics(func() {
		git.NewMustLazyGit(f.Temp)()
	})
}

func TestLazyGitRepo(t *testing.T) {
	f := NewFixture(t)
	defer f.Close()
	f.SetupGitTestRepos()

	repo := git.NewMustLazyGit(f.TestRepo.Path())
	f.Equal(f.TestRepo.Path(), repo().Path())
}

func TestGitDir(t *testing.T) {
	f := NewFixture(t)
	defer f.Close()
	f.SetupGitTestRepos()

	f.Equal(filepath.Join(f.Temp, "repo", ".git"), f.TestRepo.GitDir())
	f.Equal(filepath.Join(f.Temp, "origin.git"), f.TestOrigin.GitDir())
}

func TestRunExitingNonZeroIsAnError(t *testing.T) {
	f := NewFixture(t)
	defer f.Close()

	f.SetupGitTestRepos()

	cr, err := f.TestRepo.Run("show-ref", "--", "there/is/no/way/this/is/a/valid/ref")
	f.NotNil(cr)
	f.NotNil(err)
	f.NotNil(cr.Command())

	cfe, ok := err.(*git.CommandFailedError)
	f.True(ok)
	f.Contains(cfe.Command, "show-ref -- there/is/no/way/this/is/a/valid/ref")
	f.Equal(1, cfe.ExitCode)
	f.Empty(cfe.Stderr)
}

type (
	HelperScript struct {
		Cmd    *exec.Cmd
		Stdout bytes.Buffer
		Stderr bytes.Buffer
		f      *Fixture
	}
)

func (hs *HelperScript) Run() *HelperScript {
	hs.f.NoError(hs.Cmd.Run())
	hs.f.NotNil(hs.Cmd.ProcessState)
	hs.f.True(hs.Cmd.ProcessState.Success())
	return hs
}

func (f *Fixture) NewHelper(bin, script string, args ...string) (hs *HelperScript) {
	hs = &HelperScript{f: f}
	hs.Cmd = exec.Command(bin, append([]string{f.RootRelativeJoin(script)}, args...)...)

	hs.Cmd.Stdout = &hs.Stdout
	hs.Cmd.Stderr = io.MultiWriter(&hs.Stderr, os.Stderr)
	hs.Cmd.Env = os.Environ()
	hs.Cmd.Env = append(hs.Cmd.Env, "TEST_REPO="+f.TestRepo.Path())
	hs.Cmd.Dir = f.TestRepo.Path()

	return hs
}

func TestCatFileBlobGetsTheBytes(t *testing.T) {
	f := NewFixture(t)
	defer f.Close()

	f.SetupGitTestRepos()
	hs := f.NewHelper("bash", "internal/git/testfiles/cat_file_blob_test_setup.sh")
	hs.Run()

	// command outputs SHA1 branch-name
	output := strings.Split(strings.TrimSpace(hs.Stdout.String()), " ")

	data, err := f.TestRepo.CatFileBlob(output[1], "abc")
	f.NoError(err)
	f.Equal([]byte("abc\n"), data)
}

func TestRelPathPanicsInBareRepo(t *testing.T) {
	f := NewFixture(t)
	defer f.Close()

	f.SetupGitTestRepos()
	f.True(f.TestOrigin.IsBare())
	f.Panics(func() { f.TestOrigin.RelPath("boom.") })
}

func TestRelPathInNormalRepo(t *testing.T) {
	f := NewFixture(t)
	defer f.Close()

	f.SetupGitTestRepos()
	expect := filepath.Join(f.TestRepo.Path(), "foo", "bar")
	f.Equal(expect, f.TestRepo.RelPath("foo", "bar"))
}

type ConfigMaps struct {
	local  map[string]string
	global map[string]string
}

func collectMaps(f common.KeyValueVisitor) (cm *ConfigMaps) {
	cm = new(ConfigMaps)
	cm.global = make(map[string]string)
	cm.local = make(map[string]string)

	f(func(k, v string) error {
		switch {
		case strings.HasPrefix(k, "test.one."):
			cm.local[k] = v
		case strings.HasPrefix(k, "global.one."):
			cm.global[k] = v
		}
		return nil
	})

	return cm
}

func TestConfigVisitScope(t *testing.T) {
	expectLocal := map[string]string{"test.one.a": "a", "test.one.b": "b", "test.one.c": "c"}
	expectGlobal := map[string]string{"global.one.a": "a", "global.one.b": "b", "global.one.c": "c"}

	f := NewFixture(t)
	defer f.Close()
	f.SetupGitTestRepos()

	hs := f.NewHelper("bash", "internal/git/testfiles/config_test_setup.sh").Run()
	f.Equal("SUCCESS\n", hs.Stdout.String())

	cm := collectMaps(f.TestRepo.Config().Local().Visit)

	f.Equal(expectLocal, cm.local)
	f.Empty(cm.global)

	cm = collectMaps(f.TestRepo.Config().Global().Visit)

	f.Equal(expectGlobal, cm.global)
	f.Empty(cm.local)

	cm = collectMaps(f.TestRepo.Config().Visit)

	f.Equal(expectLocal, cm.local)
	f.Equal(expectGlobal, cm.global)

	// the origin repo should *also* see the global, but not local config
	cm = collectMaps(f.TestOrigin.Config().Visit)
	fmt.Fprintf(os.Stderr, "%s\n", f.TestOrigin.Path())

	f.Equal(expectGlobal, cm.global)
	f.Empty(cm.local)
}

type testGet struct {
	Key   string
	Ok    bool
	Val   string
	Type  string
	ErrCb func(err error)
}

func TestConfigGet(t *testing.T) {
	f := NewFixture(t)
	defer f.Close()
	f.SetupGitTestRepos()
	hs := f.NewHelper("bash", "internal/git/testfiles/config_test_setup.sh").Run()
	f.Equal("SUCCESS\n", hs.Stdout.String())

	noerr := func(err error) { f.NoError(err) }

	sayUncle := func(err error) {
		f.Error(err)
		f.Contains(err.Error(), "error: key does not contain a section: uncle")
	}

	badUnit := func(err error) {
		f.Error(err)
		f.Contains(err.Error(), "bad numeric config value 'c' for 'test.one.c' in file .git/config: invalid unit")
	}

	tests := []testGet{
		{"test.one.a", true, "a", "", noerr},
		{"test.one.b", true, "b", "", noerr},
		{"missing.key", false, "", "", noerr},
		{"uncle", false, "", "", sayUncle},
		{"test.int.a", true, "1", "int", noerr},
		{"test.one.c", false, "", "int", badUnit},
	}

	for _, test := range tests {
		cfg := f.TestRepo.Config().Local()
		if test.Type != "" {
			cfg = cfg.Type(test.Type)
		}
		v, ok, err := cfg.Get(test.Key)
		f.Equal(test.Val, v, "expected value %#v for key %#v but was %#v", test.Val, test.Key, v)
		if test.Ok {
			f.True(ok, "expected ok to be true but was false for key %#v", test.Key)
		} else {
			f.False(ok, "expected ok to be false but was true for key %#v", test.Key)
		}
		test.ErrCb(err)
	}
}

func TestConfigSet(t *testing.T) {
	f := NewFixture(t)
	defer f.Close()
	f.SetupGitTestRepos()
	hs := f.NewHelper("bash", "internal/git/testfiles/config_test_setup.sh").Run()
	f.Equal("SUCCESS\n", hs.Stdout.String())

	f.NoError(f.TestRepo.Config().Local().Set("where.its.at", "odelay"))

	var buf bytes.Buffer
	cmd := exec.Command("git", "-C", f.TestRepo.Path(), "config", "--get", "where.its.at")
	cmd.Stdout = &buf
	f.NoError(cmd.Run())
	f.True(cmd.ProcessState.Success())

	f.Equal("odelay\n", buf.String())

	// make sure it fails on invalid input
	err := f.TestRepo.Config().Local().Set("thishasnosection", "what a pity")
	f.Error(err)
	e, ok := err.(*git.CommandFailedError)
	f.True(ok)
	f.Contains(e.Stderr, "error: key does not contain a section: thishasnosection")
}

func TestTypeShouldPanicIfGivenInvalidArgument(t *testing.T) {
	f := NewFixture(t)
	defer f.Close()
	f.SetupGitTestRepos()

	f.Panics(func() { f.TestRepo.Config().Type("underpants") })
}

func TestTopLevel(t *testing.T) {
	f := NewFixture(t)
	defer f.Close()
	f.SetupGitTestRepos()

	p, err := f.TestRepo.TopLevel()
	f.NoError(err)
	f.Equal(filepath.Join(f.Temp, "repo"), p)

	p, err = f.TestOrigin.TopLevel()
	f.Empty(p)
	e, ok := err.(*git.NoWorkingTreeError)
	f.True(ok)
	f.Equal("git rev-parse --show-toplevel", e.Operation)
	f.Equal(f.TestOrigin.Path(), e.Path)
}
