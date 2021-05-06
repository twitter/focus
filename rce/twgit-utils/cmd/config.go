package cmd

import (
	"fmt"

	log "github.com/sirupsen/logrus"

	"git.twitter.biz/focus/rce/twgit-utils/internal/common"
	"git.twitter.biz/focus/rce/twgit-utils/internal/unwinder"
	"github.com/spf13/cobra"
)

func ConfigCmd(cset *cmdSetup) *cobra.Command {
	configCmd := &cobra.Command{
		Use:   "config",
		Short: "subcommands for managing a config ref",
		Long: `For convenience, it's often helpful to place git related config
in the repo itself. This usually involves an orphan branch, which is a ref
that doesn't share history with the mainline of the repo. This config can
be updated independently, and can be read out of git using the 'cat-file'
command. The subcommands of 'config' provide verbs for dealing with this.
		`,
		PersistentPreRun: cset.LogSetupHook(),
	}

	var configShowCmd = &cobra.Command{
		Use:     "show",
		Aliases: []string{"cat"},
		Short:   "show the currently active config",
		Long: `The configuration can be set with the --config flag on the command
line or with the TWGIT_CMD_CONFIG environment variable.

The show command will load the config, validate it, and display it on stdout.
	`,
		RunE: func(cc *cobra.Command, args []string) error {
			return unwinder.Run(func(unwind *unwinder.U) {
				loaded, err := cset.LoadMainConfigFile()
				unwind.Check(err)

				_, err = fmt.Fprint(cc.OutOrStdout(), loaded.String())
				unwind.Check(err)
			})
		},
	}

	var configUpdateCmd = &cobra.Command{
		Use:   "update",
		Short: "update the configured admin ref that contains twgit's config",
		RunE: func(cc *cobra.Command, args []string) error {
			return unwinder.Run(func(unwind *unwinder.U) {
				bc := cset.LoadBlobConfig()

				var updated common.UpdateStatus
				var err error

				if cset.CLI.Force {
					log.Debug("config update forced")
					updated, err = bc.ForceUpdate()
				} else {
					updated, err = bc.Update()
				}

				unwind.Check(err)
				cset.Sayln(cc, updated)
			})
		},
	}

	var configRegistry = &cobra.Command{
		Use:     "registry",
		Short:   "display the internal state of the twgit command from various sources of input",
		Aliases: []string{"reg"},
		Hidden:  true,
		RunE: func(cc *cobra.Command, args []string) error {
			_, err := fmt.Fprint(cc.ErrOrStderr())
			return err
		},
	}

	// TODO: this --config flag is currently a lie
	configCmd.PersistentFlags().StringVar(&cset.CLI.ConfigPath, "config", cset.CLI.ConfigPath, "path to the config file, also read from TWGIT_CONFIG")
	configUpdateCmd.Flags().BoolVarP(&cset.CLI.Force, "force", "f", cset.CLI.Force, "ignore TTL and try to update the ref")

	configCmd.AddCommand(configShowCmd, configUpdateCmd, configRegistry)

	return configCmd
}
