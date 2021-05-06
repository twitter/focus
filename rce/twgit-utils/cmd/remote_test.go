package cmd

import (
	"os"
	"testing"

	"git.twitter.biz/focus/rce/twgit-utils/internal/domain"
	"git.twitter.biz/focus/rce/twgit-utils/internal/git"
	"git.twitter.biz/focus/rce/twgit-utils/internal/unwinder"
)

func (f *Fixture) UnmarshalConfig() (conf *domain.Config) {
	conf, err := domain.LoadConfigFromYaml(f.ReadProjectRootRelativeFile(f.ConfigPath))
	f.NoError(err)
	return conf
}

func TestRemoteCmd(t *testing.T) {
	f := NewFixture(t)
	defer f.Close()

	os.Setenv("TWGIT_USER", "butts")

	config := f.UnmarshalConfig()

	var cmd *git.GitCmd

	err := unwinder.Run(func(u *unwinder.U) {
		cmd = createRemoteCmd(
			u, "origin", "twgit://source/archive/~user", "butts",
			config, git.ConstLazyGit(f.TestRepo.Repo),
		)
	})

	f.NoError(err)
	f.NotNil(cmd)
	f.Equal(
		cmd.Args,
		[]string{
			git.GitBin(),
			"-C",
			f.TestRepo.Path(),
			"remote-https",
			"origin",
			"https://git.twitter.biz/source-archive00/~butts",
		},
	)
}
