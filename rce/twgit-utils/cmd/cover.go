package cmd

import (
	"net/http"

	log "github.com/sirupsen/logrus"

	"github.com/spf13/cobra"
)

func CoverCmd(setup *cmdSetup) *cobra.Command {
	addr := ":8081"

	cc := &cobra.Command{
		Use:   "cover path/to/coverage/dir",
		Short: "a tiny static web server to serve the go coverage file",
		Args:  cobra.ExactArgs(1),
		RunE: func(cc *cobra.Command, args []string) error {
			fs := http.FileServer(http.Dir(args[0]))

			http.Handle("/", fs)
			log.SetOutput(cc.ErrOrStderr())
			log.Infof("listening on %s", addr)

			return http.ListenAndServe(addr, nil)
		},
		Hidden: true,
	}

	cc.Flags().StringVar(&addr, "listen", addr, "address to listen on")
	return cc
}
