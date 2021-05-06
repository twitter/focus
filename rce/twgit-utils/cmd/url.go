package cmd

import (
	"fmt"

	"git.twitter.biz/focus/rce/twgit-utils/internal/resolver"
	"git.twitter.biz/focus/rce/twgit-utils/internal/unwinder"
	"github.com/spf13/cobra"
)

const (
	UserConfigKey  = "twgit.cmd.user"
	ExplicitEnvKey = "twgit.user"
	UserEnvKey     = "env.user"
)

var UserConfigKeys []string = []string{
	UserConfigKey, ExplicitEnvKey, UserEnvKey,
}

func UrlCmd(setup *cmdSetup) *cobra.Command {
	cc := &cobra.Command{
		Use:     "url twgit://",
		Short:   "prints out the translated twgit:// url according to the config",
		Aliases: []string{"xlate"},

		Args:   cobra.ExactArgs(1),
		PreRun: setup.LogSetupHook(),
		RunE: func(cc *cobra.Command, args []string) error {
			return unwinder.Run(func(unwind *unwinder.U) {
				user := setup.CLI.User

				if user == "" {
					unwind.Errorf(
						"Could not determine username to use from either the environment or configuration. "+
							"This command uses the USER environment variable as a default value, which can be "+
							"overridden in git config by running `git config --global %s NAME`. "+
							"You can set the TWGIT_USER env var to override the settings in git config. "+
							"Lastly, the --user or -u flags can be given when directly invoking this command "+
							"has the highest precedence.",
						UserConfigKey,
					)
				}

				twgitUrlStr := args[0] // TODO: validate this

				loaded, err := setup.LoadMainConfigFile()
				unwind.Check(err)

				url, err := resolver.NewResolver(user, loaded.Config).Resolve(twgitUrlStr)
				unwind.Check(err)

				fmt.Fprintln(cc.OutOrStdout(), url.String())
			})
		},
	}

	setup.ConfigFlag(cc)
	setup.UserFlag(cc)

	return cc
}
