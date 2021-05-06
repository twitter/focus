package cmd

import (
	"fmt"
	"os"
	"testing"
)

type (
	urlTest struct {
		args        []string
		expect      string
		errNonEmpty bool
	}
)

func (u *urlTest) Out(s string) *urlTest       { u.expect = s; return u }
func (u *urlTest) ErrNonEmpty(b bool) *urlTest { u.errNonEmpty = b; return u }
func (u *urlTest) Args(s ...string) *urlTest   { u.args = append(u.args, s...); return u }

func UrlTest() *urlTest {
	return &urlTest{
		args: []string{"url"},
	}
}

func TestUrlCommand(t *testing.T) {
	f := NewFixture(t)
	defer f.Close()
	f.setupAdminRef()

	altConfigPath := f.RootRelativeJoin("cmd", "testfiles", "alt_config.yaml")

	tests := []*urlTest{
		UrlTest().Args("twgit://source/main").Out("https://git.twitter.biz/source/main\n"),
		UrlTest().Args("twgit://source/dev/~user").Out("https://git.twitter.biz/source-dev00.git/~butts\n"),
		UrlTest().Args("twgit://source/archive/_tags").Out("https://git.twitter.biz/source-archive00/_tags\n"),
		UrlTest().Args("twgit://source/archive/_all").Out("https://git.twitter.biz/source-archive00/_all\n"),
		UrlTest().Args("twgit://source/archive/~user").Out("https://git.twitter.biz/source-archive00/~butts\n"),
		UrlTest().
			Args("--user=harpo", "twgit://source/archive/~user").
			Out("https://git.twitter.biz/source-archive01/~harpo\n"),
		UrlTest().
			Args("--config="+altConfigPath, "twgit://derp/main").
			Out("https://derp.biz/derp/main\n"),
	}

	run := func(ut *urlTest) *WrappedCommand {
		defer f.ResetEnv()
		os.Setenv("TWGIT_USER", "butts")

		cc := f.WrapCommand(RootCmd)

		f.NotEmpty(cc.setup.CLI.User)

		cc.SetArgs(ut.args...)

		f.NoError(cc.Execute())

		if ut.expect != "" {
			f.Equal(ut.expect,
				cc.Stdout.String(),
				fmt.Sprintf("ut: %#v\n", ut),
			)
		}

		if ut.errNonEmpty {
			f.NotEmpty(cc.Stderr.Bytes(), fmt.Sprintf("no stderr output cmd: %#v", ut))
		}
		return cc
	}

	for _, ut := range tests {
		run(ut)
	}

	// make sure the default persistent flags are at least recognized on the cmd
	rootFlags := []string{"--debug", "--trace", "--quiet"}

	for _, rflag := range rootFlags {
		run(UrlTest().Args(rflag, "twgit://source/main").ErrNonEmpty(true))
	}

	func() {
		defer f.ResetEnv()
		os.Unsetenv("USER")
		os.Unsetenv("TWGIT_USER")
		cc := f.WrapCommand(RootCmd)
		cc.SetArgs("url", "twgit://source/main")
		f.Error(cc.Execute())
		f.Contains(cc.Stderr.String(), "Could not determine username to use from either")
	}()
}
