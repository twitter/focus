package main

import (
	"os"

	"git.twitter.biz/focus/rce/twgit-utils/cmd"
	"git.twitter.biz/focus/rce/twgit-utils/internal/backend"
	"git.twitter.biz/focus/rce/twgit-utils/internal/common"
	"git.twitter.biz/focus/rce/twgit-utils/internal/config"
	"git.twitter.biz/focus/rce/twgit-utils/internal/unwinder"
	log "github.com/sirupsen/logrus"
	"github.com/spf13/cobra"
)

func BackendCmd(env []string) *cobra.Command {
	return &cobra.Command{
		Use:   "backend",
		Short: "invokes the git-http-backend command after some mangling",
		PreRunE: func(cc *cobra.Command, args []string) (err error) {
			return unwinder.Run(func(unwind *unwinder.U) {
				cfg, err := config.NewCLIConfigFromEnv(common.NewEnvVisitor(env))
				unwind.Check(err)

				cmd.LogSetup(
					config.NewLogConfig(
						cfg,
						cc.ErrOrStderr(),
					),
				)
			})
		},
		RunE: func(cc *cobra.Command, args []string) error {
			return unwinder.Run(func(unwind *unwinder.U) {
				be, err := backend.New(
					&backend.Config{
						Env:    os.Environ(),
						Stdin:  cc.InOrStdin(),
						Stdout: cc.OutOrStdout(),
						Stderr: cc.ErrOrStderr(),
					})
				unwind.Check(err)

				cmd, err := be.Cmd()
				unwind.Check(err)

				log.WithFields(log.Fields{
					"Path": cmd.Path,
					"Args": cmd.Args,
					"Env":  cmd.Env,
				}).Debug("running backend")

				unwind.Check(cmd.Run())

				if code := cmd.ProcessState.ExitCode(); code != 0 {
					unwind.Errorf("git command %#v exited with status %d", cmd.String(), code)
				}
			})
		},
	}
}

func main() {
	if err := BackendCmd(os.Environ()).Execute(); err != nil {
		common.LogStack(err)
		log.Fatal(err)
	}
}
