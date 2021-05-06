package backend

import (
	"bytes"
	"fmt"
	"os"
	"os/exec"
	"strings"
	"testing"

	"git.twitter.biz/focus/rce/twgit-utils/internal/testutils"
	"git.twitter.biz/focus/rce/twgit-utils/internal/validation"
)

type (
	Fixture struct {
		*testutils.Fixture
	}
)

func NewFixture(t *testing.T) *Fixture {
	f := &Fixture{
		Fixture: testutils.NewFixture(t),
	}

	f.SetupGitTestRepos()

	return f
}

const GitBin = "/opt/special/bin/git"

var cgiTestEnviron = []string{
	"GIT_PROJECT_ROOT=/srv/git",
	"GIT_HTTP_EXPORT_ALL=",
	"GIT_HTTP_MAX_REQUEST_BUFFER=100M",
	"GIT_BIN=" + GitBin,
	"TWGIT_DEBUG=true",
	"HTTP_HOST=localhost",
	"HTTP_USER_AGENT=git/2.31.1",
	"HTTP_ACCEPT=*/*",
	"HTTP_ACCEPT_ENCODING=deflate, gzip",
	"HTTP_ACCEPT_LANGUAGE=C, *;q=0.9",
	"HTTP_PRAGMA=no-cache",
	"HTTP_GIT_PROTOCOL=version=2",
	"PATH=/usr/local/apache2/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
	"SERVER_SIGNATURE=",
	"SERVER_SOFTWARE=Apache/2.4.46 (Unix) OpenSSL/1.1.1d",
	"SERVER_NAME=localhost",
	"SERVER_ADDR=::1",
	"SERVER_PORT=80",
	"REMOTE_ADDR=::1",
	"DOCUMENT_ROOT=/srv/git",
	"REQUEST_SCHEME=http",
	"CONTEXT_PREFIX=/git/",
	"CONTEXT_DOCUMENT_ROOT=/opt/twgit/cgi-bin/twgit-backend/",
	"SERVER_ADMIN=gitmeister@twitter.com",
	"SCRIPT_FILENAME=/opt/twgit/cgi-bin/twgit-backend",
	"REMOTE_PORT=48566",
	"GATEWAY_INTERFACE=CGI/1.1",
	"SERVER_PROTOCOL=HTTP/1.1",
	"REQUEST_METHOD=GET",
	"QUERY_STRING=service=git-upload-pack",
	"SCRIPT_NAME=/git",
	"REQUEST_URI=/git/focus.git/@_VIEW_@/info/refs?service=git-upload-pack",
	"PATH_INFO=/focus.git/@_VIEW_@/info/refs",
	"PATH_TRANSLATED=/srv/git/focus.git/@_VIEW_@/info/refs",
}

func envCopy(view string) []string {
	c := make([]string, len(cgiTestEnviron))
	copy(c, cgiTestEnviron)
	for i := range c {
		c[i] = strings.ReplaceAll(c[i], "/@_VIEW_@/", "/"+view+"/")
	}
	return c
}

func backendConfig(env []string) *Config {
	return &Config{
		Env:    env,
		Stdin:  new(bytes.Buffer),
		Stdout: new(bytes.Buffer),
		Stderr: new(bytes.Buffer),
	}
}

func TestAllRefs(t *testing.T) {
	f := NewFixture(t)
	defer f.Close()

	cmd := f.mkCmd("_all")

	f.Equal(GitBin, cmd.Path)

	f.Equal(
		[]string{
			"git",
			"-c", "transfer.hideRefs=refs/heads",
			"-c", "transfer.hideRefs=refs/tags",
			"-c", "transfer.hideRefs=!refs/heads/master",
			"-c", "transfer.hideRefs=!refs/admin",
			"-c", "transfer.hideRefs=!refs",
			"http-backend",
		},
		cmd.Args,
	)

	f.Equal(
		[]string{
			"LC_ALL=C",
			"PATH_INFO=/focus.git/info/refs",
			"PATH_TRANSLATED=/srv/git/focus.git/info/refs",
			"REQUEST_URI=/git/focus.git/info/refs?service=git-upload-pack",
		},
		cmd.Env[len(cmd.Env)-4:],
	)
}

func TestChangeDefaultHideRefsInEnv(t *testing.T) {
	f := NewFixture(t)
	defer f.Close()

	env := envCopy("_all")
	env = append(
		env,
		"TWGIT_BACKEND_APPEND_HIDE_REFS=!refs/foo,!refs/bar",
	)

	bc, err := New(backendConfig(env))
	f.NoError(err)
	cmd, err := bc.Cmd()
	f.NoError(err)

	f.Equal(GitBin, cmd.Path)
	f.Equal(
		[]string{
			"git",
			"-c", "transfer.hideRefs=refs/heads",
			"-c", "transfer.hideRefs=refs/tags",
			"-c", "transfer.hideRefs=!refs/heads/master",
			"-c", "transfer.hideRefs=!refs/admin",
			"-c", "transfer.hideRefs=!refs/foo",
			"-c", "transfer.hideRefs=!refs/bar",
			"-c", "transfer.hideRefs=!refs",
			"http-backend",
		},
		cmd.Args,
	)
}

func (f *Fixture) mkCmd(viewName string) *exec.Cmd {
	bc, err := New(backendConfig(envCopy(viewName)))
	f.NoError(err)
	cmd, err := bc.Cmd()
	f.NoError(err)
	return cmd
}

func TestTags(t *testing.T) {
	f := NewFixture(t)
	defer f.Close()

	cmd := f.mkCmd("_tags")

	f.Equal(GitBin, cmd.Path)

	f.Equal(
		[]string{
			"git",
			"-c", "transfer.hideRefs=refs/heads",
			"-c", "transfer.hideRefs=refs/tags",
			"-c", "transfer.hideRefs=!refs/heads/master",
			"-c", "transfer.hideRefs=!refs/admin",
			"-c", "transfer.hideRefs=!refs/tags",
			"http-backend",
		},
		cmd.Args,
	)

	f.Equal(
		[]string{
			"LC_ALL=C",
			"PATH_INFO=/focus.git/info/refs",
			"PATH_TRANSLATED=/srv/git/focus.git/info/refs",
			"REQUEST_URI=/git/focus.git/info/refs?service=git-upload-pack",
		},
		cmd.Env[len(cmd.Env)-4:],
	)
}

func TestUser(t *testing.T) {
	f := NewFixture(t)
	defer f.Close()

	cmd := f.mkCmd("~youzorr")

	f.Equal(GitBin, cmd.Path)

	f.Equal(
		[]string{
			"git",
			"-c", "transfer.hideRefs=refs/heads",
			"-c", "transfer.hideRefs=refs/tags",
			"-c", "transfer.hideRefs=!refs/heads/master",
			"-c", "transfer.hideRefs=!refs/admin",
			"-c", "transfer.hideRefs=!refs/heads/youzorr",
			"http-backend",
		},
		cmd.Args,
	)

	f.Equal(
		[]string{
			"LC_ALL=C",
			"PATH_INFO=/focus.git/info/refs",
			"PATH_TRANSLATED=/srv/git/focus.git/info/refs",
			"REQUEST_URI=/git/focus.git/info/refs?service=git-upload-pack",
		},
		cmd.Env[len(cmd.Env)-4:],
	)
}

func unsetEnv(env []string, k string) []string {
	trimmed := make([]string, len(env))
	sk := k + "="

	for _, v := range env {
		if !strings.HasPrefix(v, sk) {
			trimmed = append(trimmed, v)
		}
	}

	return trimmed
}

func TestUseHardcodedDefaultGitBinIfNotInEnv(t *testing.T) {
	f := NewFixture(t)
	defer f.Close()

	env := envCopy("_all")
	env = unsetEnv(env, "GIT_BIN")

	be, err := New(backendConfig(env))
	f.NoError(err)
	cmd, err := be.Cmd()
	f.NoError(err)

	f.Equal(cmd.Path, "/usr/bin/git")
}

func TestErrorIfRequiredVarsNotSet(t *testing.T) {
	f := NewFixture(t)
	defer f.Close()

	run := func(k string) {
		env := envCopy("_all")
		env = unsetEnv(env, k)

		be, err := New(backendConfig(env))
		f.NoError(err)
		cmd, err := be.Cmd()
		f.Nil(cmd)
		f.Error(err, "no error for %#v", k)
		fmt.Fprintln(os.Stderr, validation.SprintValidationErrors(err, nil))
		f.Equal(
			fmt.Sprintf("The following env vars were expected to be set but were not: %s", k), err.Error(),
		)
	}

	required := []string{"PATH_INFO", "PATH_TRANSLATED", "REQUEST_URI"}

	for _, k := range required {
		run(k)
	}
}
