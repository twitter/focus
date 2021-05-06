package cmd

import (
	"bytes"
	"os"
	"testing"

	"git.twitter.biz/focus/rce/twgit-utils/internal/common"
	"git.twitter.biz/focus/rce/twgit-utils/internal/config"
	"git.twitter.biz/focus/rce/twgit-utils/internal/git"
	"git.twitter.biz/focus/rce/twgit-utils/internal/testutils"
	"github.com/spf13/cobra"
)

type ()

type (
	Fixture struct {
		*testutils.Fixture
	}

	WrappedCommand struct {
		*cobra.Command
		Stdin  bytes.Buffer
		Stdout bytes.Buffer
		Stderr bytes.Buffer
		setup  cmdSetup
	}
)

func (wc *WrappedCommand) SetArgs(args ...string)  { wc.Command.SetArgs(args) }
func (wc *WrappedCommand) ParseFlags(fl ...string) { wc.Command.ParseFlags(fl) }

func NewFixture(t *testing.T) *Fixture {
	f := &Fixture{
		Fixture: testutils.NewFixture(t),
	}

	f.SetupGitTestRepos()

	return f
}

func (f *Fixture) setupAdminRef() {
	cr, err := f.TestRepo.Run("fetch", "origin", "refs/admin/twgit:refs/admin/twgit")
	f.NoError(err)
	f.Equal(0, cr.ExitCode())
}

func (f *Fixture) WrapCommand(constructor func(*cmdSetup) *cobra.Command) *WrappedCommand {
	wc := &WrappedCommand{}

	wc.setup = cmdSetup{
		Repo: git.ConstLazyGit(f.TestRepo.Repo),
		env:  os.Environ(),
	}

	config.CLIConfigDefaults(&wc.setup.CLI)
	config.LoadCLIConfigFromEnv(common.NewEnvVisitor(wc.setup.env), &wc.setup.CLI)

	wc.Command = constructor(&wc.setup)

	wc.SetIn(&wc.Stdin)
	wc.SetOut(&wc.Stdout)
	wc.SetErr(&wc.Stderr)
	return wc
}
