package cmd

import (
	"os"
	"strings"

	"git.twitter.biz/focus/rce/twgit-utils/internal/domain"
	"git.twitter.biz/focus/rce/twgit-utils/internal/git"
	"git.twitter.biz/focus/rce/twgit-utils/internal/resolver"
	"git.twitter.biz/focus/rce/twgit-utils/internal/unwinder"
	"github.com/spf13/cobra"
)

func createRemoteCmd(
	unwind *unwinder.U,
	remote, url, user string,
	config *domain.Config,
	repo git.LazyGit,
) (cmd *git.GitCmd) {
	_, err := domain.ParseTwgitURL(url)
	unwind.Check(err)

	resolver := resolver.NewResolver(user, config)

	realURL, err := resolver.Resolve(url)
	unwind.Check(err)

	cmd, err = repo().Cmd()
	unwind.Check(err)

	cmd.AddArgf("remote-%s", realURL.Scheme)
	if strings.Contains(remote, "://") {
		cmd.AddArgs(realURL.String())
	} else {
		cmd.AddArgs(remote)
	}
	cmd.AddArgs(realURL.String())
	return cmd
}

func RemoteCmd(cset *cmdSetup) *cobra.Command {
	cc := &cobra.Command{
		Use:   "remote url|remote-name url",
		Short: "invoked by git as git-remote-twgit, resolves url and makes backend connection",
		Args:  cobra.ExactArgs(2),

		PreRun: cset.LogSetupHook(),
		RunE: func(cc *cobra.Command, args []string) error {
			return unwinder.Run(func(unwind *unwinder.U) {
				remote := args[0]
				url := args[1]

				loaded, err := cset.LoadMainConfigFile()
				unwind.Check(err)

				// this allows us to validate that the command is created properly in tests
				// since actually running the command would require spinning up an http server
				cmd := createRemoteCmd(unwind, remote, url, cset.CLI.User, loaded.Config, cset.Repo)

				cmd.Cmd.Stdin = os.Stdin
				cmd.Cmd.Stdout = os.Stdout
				// don't set Stderr here because it's a MultiWriter that collects
				// stderr output and also writes to os.Stderr. It's helpful for the
				// cmd to determine if an error occurred

				unwind.Check(cmd.Run())

				if code := cmd.ProcessState.ExitCode(); code != 0 {
					unwind.Errorf("git command %#v failed, exitstatus: %d", cmd.CommandLine(), code)
				}
			})
		},
	}

	return cc
}
