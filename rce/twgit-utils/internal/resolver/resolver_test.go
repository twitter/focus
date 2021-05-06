package resolver

import (
	"testing"

	"git.twitter.biz/focus/rce/twgit-utils/internal/domain"
	"git.twitter.biz/focus/rce/twgit-utils/internal/testutils"
	"git.twitter.biz/focus/rce/twgit-utils/internal/userhash"
)

type (
	Fixture struct {
		*testutils.Fixture
		cnf *domain.Config
	}
)

const Username = "youzorr"

func NewFixture(t *testing.T) *Fixture {
	f := &Fixture{Fixture: testutils.NewFixture(t)}
	f.cnf = f.UnmarshalConfig()
	return f
}

func (f *Fixture) UnmarshalConfig() (conf *domain.Config) {
	conf, err := domain.LoadConfigFromYaml(f.ReadProjectRootRelativeFile(f.ConfigPath))
	f.NoError(err)
	return conf
}

type constantHasher struct {
	user  string
	value string
}

func (c *constantHasher) Locate(n string) string {
	c.user = n
	return c.value
}

var _ userhash.Hasher = new(constantHasher)

type ResolveTestInput struct {
	urlstr         string
	p              *domain.Profile
	hasher         userhash.Hasher
	user           string
	expected       string
	errMsgContains string
}

func (f *Fixture) runResolverTestE(r *ResolveTestInput) (result string, err error) {
	f.True(
		r.expected != "" || r.errMsgContains != "",
		"either expected or errMsgContains must be a non-blank string",
	)

	rslv := &resolver{user: r.user}
	rslv.config = &domain.Config{
		Version: 1,
		Repos: []domain.Repo{
			{
				Name:     r.p.Name,
				Profiles: []domain.Profile{*r.p},
			},
		},
	}

	domain.PostUnmarshal(rslv.config)

	rslv.mkHasher = func(_ userhash.HasherConfig) (userhash.Hasher, error) { return r.hasher, nil }

	u, err := rslv.Resolve(r.urlstr)
	if err != nil {
		return "", err
	}

	return u.String(), nil
}

func (f *Fixture) runResolveTest(r *ResolveTestInput) {
	f.True(
		r.expected != "" || r.errMsgContains != "",
		"either expected or errMsgContains must be a non-blank string",
	)

	result, err := f.runResolverTestE(r)
	if r.errMsgContains != "" {
		f.NotNil(err)
		f.Contains(err.Error(), r.errMsgContains)
	} else {
		f.NoError(err)
		f.Equal(r.expected, result)
	}
}

func TestResolve(t *testing.T) {
	f := NewFixture(t)
	defer f.Close()

	f.runResolveTest(
		&ResolveTestInput{
			urlstr: "twgit://source/dev/~user",
			p: &domain.Profile{
				Name:         "source",
				Role:         "dev",
				Backend:      "{{.Scheme}}://{{.Host}}/{{.Repo}}-{{.Node}}.git/{{.View}}{{.Options}}",
				Nodes:        []string{"dev01", "dev02", "dev03"},
				Views:        []string{"~user"},
				DefaultView:  "",
				Scheme:       "https",
				Host:         "git.twitter.biz",
				ReadonlyHost: "ro-git.twitter.biz",
			},
			hasher:         &constantHasher{value: "dev01"},
			user:           Username,
			expected:       "https://git.twitter.biz/source-dev01.git/~youzorr",
			errMsgContains: "",
		},
	)

	f.runResolveTest(
		&ResolveTestInput{
			urlstr: "twgit://source/archive/_tags",
			p: &domain.Profile{
				Name:         "source",
				Host:         "git.twitter.biz",
				ReadonlyHost: "ro-git.twitter.biz",
				Scheme:       "https",
				Role:         "archive",
				Backend:      "{{.Scheme}}://{{.Host}}/{{.Repo}}-{{.Node}}.git/{{.View}}{{.Options}}",
				Nodes:        []string{"archive00", "archive01"},
				Views:        []string{"_tags"},
				DefaultView:  "",
			},
			hasher:         &constantHasher{value: "archive00"},
			user:           Username,
			expected:       "https://git.twitter.biz/source-archive00.git/_tags",
			errMsgContains: "",
		},
	)

	f.runResolveTest(
		&ResolveTestInput{
			urlstr: "twgit://source/main",
			p: &domain.Profile{
				Name:         "source",
				Role:         "main",
				Host:         "git.twitter.biz",
				ReadonlyHost: "ro-git.twitter.biz",
				Scheme:       "https",
				Backend:      "{{.Scheme}}://{{.Host}}/{{.Repo}}",
				Nodes:        []string{},
				Views:        []string{},
				DefaultView:  "",
			},
			hasher:         nil,
			user:           "",
			expected:       "https://git.twitter.biz/source",
			errMsgContains: "",
		},
	)
}

func TestResolveNilHasherWithNonZeroNodes(t *testing.T) {
	f := NewFixture(t)
	defer f.Close()

	f.runResolveTest(
		&ResolveTestInput{
			urlstr: "twgit://source/dev/~user",
			p: &domain.Profile{
				Name:         "source",
				Role:         "dev",
				Host:         "git.twitter.biz",
				ReadonlyHost: "ro-git.twitter.biz",
				Scheme:       "https",
				Backend:      "{{.Scheme}}://{{.Host}}/{{.Repo}}-{{.Node}}.git/{{.View}}{{.Options}}",
				Nodes:        []string{"dev01", "dev02", "dev03"},
				Views:        []string{"~user"},
				DefaultView:  "",
			},
			hasher:         nil, /* nil hasher with multiple Nodes causes an error*/
			user:           Username,
			expected:       "",
			errMsgContains: "[BUG] For profile",
		},
	)
}

func TestResolveMultipleNodesButNoUsernameError(t *testing.T) {
	f := NewFixture(t)
	defer f.Close()

	f.runResolveTest(
		&ResolveTestInput{
			urlstr: "twgit://source/dev/~user",
			p: &domain.Profile{
				Name:         "source",
				Role:         "dev",
				Host:         "git.twitter.biz",
				ReadonlyHost: "ro-git.twitter.biz",
				Scheme:       "https",
				Backend:      "{{.Scheme}}://{{.Host}}/{{.Repo}}-{{.Node}}.git/{{.View}}{{.Options}}",
				Nodes:        []string{"dev01", "dev02", "dev03"},
				Views:        []string{"~user"},
				DefaultView:  "",
			},
			hasher:         &constantHasher{value: "dev01"},
			user:           "", /* blank user causes an error here */
			expected:       "",
			errMsgContains: "however a username was not provided",
		},
	)
}

func TestProfileDoesNotHaveDesiredViewName(t *testing.T) {
	f := NewFixture(t)
	defer f.Close()

	f.runResolveTest(
		&ResolveTestInput{
			urlstr: "twgit://source/dev/~user", /* <-- the requested view ~user */
			p: &domain.Profile{
				Name:         "source",
				Role:         "dev",
				Host:         "git.twitter.biz",
				ReadonlyHost: "ro-git.twitter.biz",
				Scheme:       "https",

				Backend:     "{{.Scheme}}://{{.Host}}/{{.Repo}}-{{.Node}}.git/{{.View}}{{.Options}}",
				Nodes:       []string{"dev01", "dev02", "dev03"},
				Views:       []string{"all"}, /* <-- doesn't match any of these views */
				DefaultView: "",
			},
			hasher:         &constantHasher{value: "dev01"},
			user:           Username,
			expected:       "",
			errMsgContains: "does not have a configured view \"~user\"",
		},
	)
}

func TestSchemeNotDefinedInProfileOrDefaults(t *testing.T) {
	f := NewFixture(t)
	defer f.Close()

	f.runResolveTest(
		&ResolveTestInput{
			urlstr: "twgit://source/dev/~user",
			p: &domain.Profile{
				Name:         "source",
				Role:         "dev",
				Backend:      "{{.Scheme}}://{{.Host}}/{{.Repo}}-{{.Node}}.git/{{.View}}{{.Options}}",
				Nodes:        []string{"dev01", "dev02", "dev03"},
				Views:        []string{"~user"},
				DefaultView:  "",
				Scheme:       "", /* <-- not defined in profile */
				Host:         "git.twitter.biz",
				ReadonlyHost: "ro-git.twitter.biz",
			},
			hasher:         &constantHasher{value: "dev01"},
			user:           Username,
			expected:       "",
			errMsgContains: "did not have a scheme defined in either",
		},
	)
}

func TestOptionsArePassedThrough(t *testing.T) {
	f := NewFixture(t)
	defer f.Close()

	f.runResolveTest(
		&ResolveTestInput{
			urlstr: "twgit://source/dev/~user?a=b&c=d",
			p: &domain.Profile{
				Name:         "source",
				Role:         "dev",
				Host:         "git.twitter.biz",
				ReadonlyHost: "ro-git.twitter.biz",
				Scheme:       "https",
				Backend:      "{{.Scheme}}://{{.Host}}/{{.Repo}}-{{.Node}}.git/{{.View}}{{.Options}}",
				Nodes:        []string{"dev01", "dev02", "dev03"},
				Views:        []string{"~user"},
				DefaultView:  "",
			},
			hasher:         &constantHasher{value: "dev01"},
			user:           Username,
			expected:       "https://git.twitter.biz/source-dev01.git/~youzorr?a=b&c=d",
			errMsgContains: "",
		},
	)
}

func TestTemplateErrorsBarfCorrectly(t *testing.T) {
	f := NewFixture(t)
	defer f.Close()

	f.runResolveTest(
		&ResolveTestInput{
			urlstr: "twgit://source/dev/~user",
			p: &domain.Profile{
				Name:         "source",
				Role:         "dev",
				Host:         "git.twitter.biz",
				ReadonlyHost: "ro-git.twitter.biz",
				Scheme:       "https",
				Backend:      "{{.Scheme}}://{{.Host}}/{{.Repo}}-{{.Node}}.git/{{.WHATTHEHELLISTHISTHING}}",
				Nodes:        []string{"dev01", "dev02", "dev03"},
				Views:        []string{"~user"},
				DefaultView:  "",
			},
			hasher:         &constantHasher{value: "dev01"},
			user:           Username,
			expected:       "",
			errMsgContains: "WHATTHEHELLISTHISTHING",
		},
	)
}

func TestResultingURLParseErrorsHandledCorrectly(t *testing.T) {
	f := NewFixture(t)
	defer f.Close()

	f.runResolveTest(
		&ResolveTestInput{
			urlstr: "twgit://source/dev/~user",
			p: &domain.Profile{
				Name:         "source",
				Role:         "dev",
				Host:         "git.twitter.biz",
				ReadonlyHost: "ro-git.twitter.biz",
				Scheme:       "https",
				Backend:      "{{.Scheme}}://{{.Host}}/{{.Repo}}-{{.Node}}.git/\x00",
				Nodes:        []string{"dev01", "dev02", "dev03"},
				Views:        []string{"~user"},
				DefaultView:  "",
			},
			hasher:         &constantHasher{value: "dev01"},
			user:           Username,
			expected:       "",
			errMsgContains: "which was not parsable by Go's url parsing code.",
		},
	)
}
