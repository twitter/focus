package cmd

import (
	"bufio"
	"fmt"

	"git.twitter.biz/focus/rce/twgit-utils/internal/unwinder"
	"git.twitter.biz/focus/rce/twgit-utils/internal/userhash"
	"github.com/davecgh/go-spew/spew"
	"github.com/spf13/cobra"
)

func HashCmd(setup *cmdSetup) *cobra.Command {
	var userHashCmd = &cobra.Command{
		Use:   "hash [--repo=REPO] [--role=ROLE] user...|-",
		Short: "map users to dev servers",
		Long: `users are assigned to a copy of the repo on one of
a number of dev servers. This command takes a list of users on the commnad line
or a single '-' to indicate reading from stdin, and outputs the user name
followed by a tab, followed by the server they are assigned to according to
a consistent hashing algorithm
		`,
		Args:   cobra.MinimumNArgs(1),
		PreRun: setup.LogSetupHook(),
		RunE: func(cc *cobra.Command, args []string) error {
			return unwinder.Run(func(unwind *unwinder.U) {
				spew.Fprintf(cc.ErrOrStderr(), "CLI config:\n%#+v\n", setup.CLI)
				loadedConfig, err := setup.LoadMainConfigFile()
				unwind.Check(err)

				cfg := loadedConfig.Config

				// find the profile the user selected
				profile, err := cfg.Find(setup.CLI.Repo, setup.CLI.Role)
				unwind.Check(err)

				// create a hasher config from the registry plus the Nodes
				// in the profile they selected
				hc, err := cfg.HasherConfig(profile.Nodes)
				unwind.Check(err)

				// create a HashRing
				hr, err := userhash.New(*hc)
				unwind.Check(err)

				prnt := func(user, host string) {
					fmt.Fprintf(cc.OutOrStdout(), "%s\t%s\n", user, host)
				}

				if len(args) == 1 && args[0] == "-" {
					scanner := bufio.NewScanner(cc.InOrStdin())

					for scanner.Scan() {
						username := scanner.Text()
						if username == "" {
							continue
						}
						host := hr.Locate(username)
						prnt(username, host)
					}
				} else {
					for _, username := range args {
						prnt(username, hr.Locate(username))
					}
				}
			})
		},
	}

	userHashCmd.Flags().IntVar(&setup.CLI.Hasher.PartitionCount, "partitioncount", setup.CLI.Hasher.PartitionCount, "")
	userHashCmd.Flags().IntVar(&setup.CLI.Hasher.ReplicationFactor, "replicationfactor", setup.CLI.Hasher.ReplicationFactor, "")
	userHashCmd.Flags().Float64Var(&setup.CLI.Hasher.Load, "load", setup.CLI.Hasher.Load, "")

	setup.RepoFlag(userHashCmd)
	setup.RoleFlag(userHashCmd)
	setup.ConfigFlag(userHashCmd)

	return userHashCmd
}
