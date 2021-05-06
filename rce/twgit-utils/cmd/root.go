package cmd

import (
	"os"

	"git.twitter.biz/focus/rce/twgit-utils/internal/common"
	"git.twitter.biz/focus/rce/twgit-utils/internal/config"
	"git.twitter.biz/focus/rce/twgit-utils/internal/git"
	"github.com/spf13/cobra"
)

func RootCmd(setup *cmdSetup) *cobra.Command {
	if setup == nil {
		repo := git.NewMustLazyGit(".")
		setup = &cmdSetup{
			Repo: repo,
			env:  os.Environ(),
		}
		config.CLIConfigDefaults(&setup.CLI)
		config.LoadCLIConfigFromEnv(common.NewEnvVisitor(setup.env), &setup.CLI)
	}

	rootCmd := &cobra.Command{
		Use:     "twgit",
		Short:   "utility for client side routing and user pleasure",
		Aliases: []string{},
	}

	rootCmd.PersistentFlags().BoolVarP(&setup.CLI.Debug, "debug", "D", setup.CLI.Debug, "increase verboseness")
	rootCmd.PersistentFlags().BoolVar(&setup.CLI.Trace, "trace", setup.CLI.Trace, "highest level of verbosity")
	rootCmd.PersistentFlags().BoolVarP(&setup.CLI.Quiet, "quiet", "q", setup.CLI.Quiet, "operate silently if there are no errors")

	cmds := []*cobra.Command{
		ConfigCmd(setup),
		HashCmd(setup),
		UrlCmd(setup),
		RemoteCmd(setup),
		CoverCmd(setup),
	}

	rootCmd.AddCommand(cmds...)

	return rootCmd
}
