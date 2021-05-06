package domain

import (
	"bytes"
	"net/url"
	"testing"
)

func TestUnmarshalTwgitYamlFile(t *testing.T) {
	f := NewFixture(t)
	defer f.Close()

	conf := f.UnmarshalConfig()

	f.Equal(1, conf.Version)
	f.Len(conf.Repos, 2)

	repo := conf.Repos[0]

	f.Equal("source", repo.Name)
	f.Equal(
		Default{
			Host:         "git.twitter.biz",
			ReadonlyHost: "rogit.twitter.biz",
			Scheme:       "https",
		},
		repo.Defaults,
	)

	f.Len(repo.Profiles, 3)

	f.Equal(
		Profile{
			Name:         "source",
			Role:         "main",
			Backend:      "{{.Scheme}}://{{.Host}}/{{.Repo}}/{{.Role}}",
			Nodes:        nil,
			Views:        nil,
			Host:         "git.twitter.biz",
			ReadonlyHost: "rogit.twitter.biz",
			Scheme:       "https",
		},
		repo.Profiles[0],
	)

	f.Equal(
		Profile{
			Name:         "source",
			Role:         "dev",
			Backend:      "{{.Scheme}}://{{.Host}}/{{.Repo}}-{{.Node}}.git/{{.View}}",
			Nodes:        []string{"dev00", "dev01", "dev02", "dev03"},
			Views:        []string{"~user"},
			Host:         "git.twitter.biz",
			ReadonlyHost: "rogit.twitter.biz",
			Scheme:       "https",
		},
		repo.Profiles[1],
	)

	f.Equal(
		Profile{
			Name:         "source",
			Role:         "archive",
			Backend:      "{{.Scheme}}://{{.Host}}/{{.Repo}}-{{.Node}}/{{.View}}{{.Options}}",
			Nodes:        []string{"archive00", "archive01"},
			Views:        []string{"_tags", "~user", "_all"},
			DefaultView:  "~user",
			Host:         "git.twitter.biz",
			ReadonlyHost: "rogit.twitter.biz",
			Scheme:       "https",
		},
		repo.Profiles[2],
	)

	f.Equal(
		Hasher{
			PartitionCount:    9973,
			ReplicationFactor: 34,
			Load:              1.25,
		},
		conf.Hasher,
	)
}

func TestViewOrDefault(t *testing.T) {
	f := NewFixture(t)
	defer f.Close()
	conf := f.UnmarshalConfig()
	repo := conf.Repos[0]
	f.Equal("source", repo.Name)
	p := repo.Profiles[2]
	f.Equal("archive", p.Role)

	f.Equal("~user", p.ViewOrDefault(""))
	f.Equal("_all", p.ViewOrDefault("_all"))
	// this method does *not* validate
	f.Equal("monkeys", p.ViewOrDefault("monkeys"))
}

func TestHasView(t *testing.T) {
	f := NewFixture(t)
	defer f.Close()
	conf := f.UnmarshalConfig()

	p := conf.Repos[0].Profiles[2]
	f.True(p.HasView("~user"))
	f.False(p.HasView("invalid"))
}

func TestHasherConfig(t *testing.T) {
	f := NewFixture(t)
	defer f.Close()
	conf := f.UnmarshalConfig()

	hc, err := conf.HasherConfig([]string{"dev00", "dev01", "dev02", "dev03"})
	f.NoError(err)
	f.Equal(1.25, hc.Load)
	f.Equal(9973, hc.PartitionCount)
	f.Equal(34, hc.ReplicationFactor)
	f.Equal([]string{"dev00", "dev01", "dev02", "dev03"}, hc.Nodes)
}

func TestProfileTemplate(t *testing.T) {
	f := NewFixture(t)
	defer f.Close()

	p := &Profile{
		Name:    "repo",
		Role:    "main",
		Backend: "https://{{.Host}}/{{.Repo}}/{{.Node}}/{{.View}}",
	}

	tpl, err := p.Template()
	f.NoError(err)
	f.NotNil(tpl)

	f.Equal("repo-main-backend", tpl.Name())

	var buf bytes.Buffer
	err = tpl.Execute(&buf,
		struct {
			Host string
			Repo string
			Node string
			View string
		}{
			Host: "example.com",
			Repo: "repo",
			Node: "node",
			View: "view",
		},
	)
	f.NoError(err)
	f.Equal("https://example.com/repo/node/view", buf.String())
}

func TestFindProfile(t *testing.T) {
	f := NewFixture(t)
	defer f.Close()
	conf := f.UnmarshalConfig()

	p, err := conf.Find("source", "dev")
	f.NoError(err)
	f.NotNil(p)
	f.NotNil(p.Views)
	f.NotEmpty(p.Views)
	f.NotNil(p.Nodes)
	f.NotEmpty(p.Nodes)

	f.Equal("source", p.Name)
	f.Equal("dev", p.Role)
}

func TestFindProfileNotFound(t *testing.T) {
	f := NewFixture(t)
	defer f.Close()
	conf := f.UnmarshalConfig()

	fr, err := conf.Find("noway", "nope")
	f.Nil(fr)
	f.Error(err)

	fr = conf.FindProfile("source", "invalid")
	f.Nil(fr)
}

func TestNewTwgitURL(t *testing.T) {
	f := NewFixture(t)
	defer f.Close()

	var err error

	twg, err := ParseTwgitURL("twgit://foo/bar/baz?a=b&c=d&c=e")
	f.NoError(err)
	f.Equal("foo", twg.Repo())
	f.Equal("bar", twg.Role())
	f.Equal("baz", twg.View())

	f.Equal(
		url.Values(map[string][]string{
			"a": {"b"},
			"c": {"d", "e"},
		}),
		twg.Options(),
	)

	_, err = ParseTwgitURL("https://foo/bar/baz")
	f.Error(err)
	f.Contains(err.Error(), "does not have a 'twgit' scheme")

	_, err = ParseTwgitURL("twgit://foo")
	f.Error(err)
	f.Contains(err.Error(), "must have at least one path component specifying the role")

	twg, err = ParseTwgitURL("twgit://foo/bar")
	f.NoError(err)
	f.Equal("", twg.View())

	_, err = ParseTwgitURL("this is invalid \x00")
	f.Error(err)
}
