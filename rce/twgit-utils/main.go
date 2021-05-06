package main

import (
	"os"
	"path"
	"path/filepath"
	"strings"

	"git.twitter.biz/focus/rce/twgit-utils/cmd"
	log "github.com/sirupsen/logrus"
)

// when we use the argv[0] name to invoke a certain command,
// this function rewrites the os.Args slice so that it contains
// the correct command as if it were run from the command-line.
// For example, if this binary is invoked as 'git-tw-backend',
// we rewrite the command as if 'twgit backend ..' was given.
// In the case of Argv[0] being 'git-tw-backend', one would call
// rewriteArgvCmd("backend").
//
// For subcommands, if we are invoked as git-tw-url-xlate, we'll
// convert that to "twgit url xlate ..."
func rewriteArgvCmd(cmd string) {
	var args []string

	cmds := strings.SplitN(cmd, "-", -1)

	args = append(args, filepath.Join(filepath.Dir(os.Args[0]), "twgit"))
	args = append(args, cmds...)
	args = append(args, os.Args[1:]...)

	os.Args = args
}

const GitBinPrefix = "git-tw-"

func main() {
	cmd := cmd.RootCmd(nil)

	basename := path.Base(os.Args[0])

	switch basename {
	case "git-remote-twgit":
		rewriteArgvCmd("remote")
	default:
		// not sure this is gonna be terribly useful but maybe?
		if strings.HasPrefix(basename, GitBinPrefix) {
			realCmd := basename[len(GitBinPrefix):]
			rewriteArgvCmd(realCmd)
		}
	}

	if err := cmd.Execute(); err != nil {
		log.Fatal(err)
	}
}
