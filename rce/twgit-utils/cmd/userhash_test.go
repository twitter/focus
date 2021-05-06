package cmd

import (
	"os"
	"testing"

	"git.twitter.biz/focus/rce/twgit-utils/internal/config"
	"git.twitter.biz/focus/rce/twgit-utils/internal/userhash"
)

func TestHashCmdFlagsAreWired(t *testing.T) {
	f := NewFixture(t)
	defer f.Close()

	cc := f.WrapCommand(RootCmd)

	cc.setup.CLI.User = "jdonham"

	configPath := f.RootRelativeJoin("config", "twgit.yaml")

	cc.SetArgs(
		"hash",
		"--config="+configPath,
		"--partitioncount=1337",
		"--replicationfactor=37",
		"--load=1.87",
		"--repo=source",
		"--role=archive",
		"bdylan",
	)

	f.NoError(cc.Execute())

	f.Equal(
		config.CLIConfig{
			ConfigPath: configPath,
			Hasher: userhash.HasherConfig{
				PartitionCount:    1337,
				ReplicationFactor: 37,
				Load:              1.87,
			},
			User: "jdonham",
			Repo: "source",
			Role: "archive",
			Test: true,
		},
		cc.setup.CLI,
	)

	f.Equal("bdylan\tarchive01\n", cc.Stdout.String())
}

func TestReadingFromStdin(t *testing.T) {
	f := NewFixture(t)
	defer f.Close()

	os.Setenv("TWGIT_CONFIG", f.RootRelativeJoin("config", "twgit.yaml"))
	cc := f.WrapCommand(RootCmd)

	cc.SetArgs(
		"hash",
		"--repo=source",
		"--role=dev",
		"-",
	)

	cc.Stdin.WriteString("jsimms\ngmac\nwil\n")

	f.NoError(cc.Execute())

	f.Equal("jsimms\tdev03\ngmac\tdev02\nwil\tdev01\n", cc.Stdout.String())
}
