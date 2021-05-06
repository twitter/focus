package config

import (
	"testing"

	"git.twitter.biz/focus/rce/twgit-utils/internal/domain"
	"git.twitter.biz/focus/rce/twgit-utils/internal/testutils"
)

type (
	Fixture struct {
		*testutils.Fixture
	}
)

const ConfigPath = "config/twgit.yaml"

func NewFixture(t *testing.T) (f *Fixture) {
	return &Fixture{
		Fixture: testutils.NewFixture(t),
	}
}

func (f *Fixture) SetupGitTestRepos() *Fixture {
	f.Fixture.SetupGitTestRepos()
	return f
}

func (f *Fixture) UnmarshalConfig() (conf *domain.Config) {
	conf, err := domain.LoadConfigFromYaml(f.ReadProjectRootRelativeFile(f.ConfigPath))
	f.NoError(err)
	return conf
}
