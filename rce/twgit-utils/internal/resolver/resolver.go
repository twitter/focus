package resolver

import (
	"bytes"
	"net/url"
	"text/template"

	"git.twitter.biz/focus/rce/twgit-utils/internal/domain"
	"git.twitter.biz/focus/rce/twgit-utils/internal/userhash"
	"github.com/pkg/errors"
)

const TwgitUrlUserKey = "twgit.url.user"

type (
	Resolver interface {
		Resolve(urlstr string) (*url.URL, error)
	}

	resolver struct {
		config *domain.Config
		user   string

		// this allows us to stub out the hasher in testng
		mkHasher func(userhash.HasherConfig) (userhash.Hasher, error)
	}
)

var _ Resolver = new(resolver)

func NewResolver(user string, c *domain.Config) (re Resolver) {
	return &resolver{
		config:   c,
		user:     user,
		mkHasher: userhash.New,
	}
}

func (r *resolver) Resolve(s string) (u *url.URL, err error) {
	tw, err := domain.ParseTwgitURL(s)
	if err != nil {
		return nil, err
	}

	p, err := r.config.Find(tw.Repo(), tw.Role())
	if err != nil {
		return nil, err
	}

	hconf, err := r.config.HasherConfig(p.Nodes)
	if err != nil {
		return nil, err
	}

	var hasher userhash.Hasher

	// if the profile has multiple nodes associated with it
	// then it'll return a non-nil HasherConfig, and we can
	// create a hasher from that
	if hconf != nil {
		hasher, err = r.mkHasher(*hconf)
		if err != nil {
			return nil, err
		}
	}

	t, err := p.Template()
	if err != nil {
		return nil, err
	}

	return Resolve(tw, p, hasher, t, r.user)
}

// Resolve takes a TwgitURL, Profile, Hasher, Template and username and if all the necessary
// information is found, returns a url.Url that the twgit:// url refers to.
func Resolve(
	tw *domain.TwgitURL,
	p *domain.Profile,
	hasher userhash.Hasher,
	tpl *template.Template,
	username string,
) (u *url.URL, err error) {
	vars := &domain.BackendTemplateVars{
		Host: p.Host,
		Repo: tw.Repo(),
		Role: tw.Role(),
	}

	if len(p.Nodes) > 0 && hasher == nil {
		return nil, errors.Errorf(
			"[BUG] For profile repo %#v, role %#v, there were nodes defined but hasher was nil. "+
				"This is most likely programmer error.",
			p.Name, p.Role,
		)
	}

	vars.Node = ""
	if hasher != nil && len(p.Nodes) > 0 {
		if username == "" {
			return nil, errors.Errorf(
				"The profile for repo %#v, role %#v has multiple backend nodes associated with it. "+
					"This means that we need to hash the username in order to determine which node a given user "+
					"is assigned to, however a username was not provided. Please set the TWGIT_URL_USER env var, "+
					"configure it in git with `git config --local twgit.url.user $USER` or provide the --user flag "+
					"on the command-line (if appropriate).",
				tw.Repo(), tw.Role(),
			)
		} else {
			vars.Node = hasher.Locate(username)
		}
	}

	if p.HasAnyViews() && !p.HasView(tw.View()) && p.DefaultView == "" {
		return nil, errors.Errorf(
			"The profile for repo %#v, role %#v does not have a configured view %#v "+
				"or a default view defined. "+
				"The provided URL was (%#v). You can use `git tw-config show` to display the "+
				"current configuration.",
			tw.Repo(), tw.Role(), tw.View(), tw.URL.String(),
		)
	}

	vars.View = p.ViewOrDefault(tw.View())
	switch {
	case vars.View == "~user":
		vars.View = "~" + username
	case vars.View == "":
		vars.View = p.DefaultView
	}

	if vars.Scheme = p.Scheme; vars.Scheme == "" {
		return nil, errors.Errorf(
			"The repo %#v, role %#v did not have a scheme defined in either its "+
				"profile or defaults section. Please check the definition in the config file "+
				"and add an appropriate value. (Hint: try 'https')",
			p.Name, p.Role,
		)
	}

	vars.Options = ""
	if len(tw.Options()) > 0 {
		vars.Options = "?" + tw.Options().Encode()
	}

	var buf bytes.Buffer
	if err := tpl.Execute(&buf, vars); err != nil {
		return nil, err
	}
	resolvedStr := buf.String()

	u, err = url.Parse(resolvedStr)
	if err != nil {
		return nil, errors.Wrapf(err,
			"The url %#v was resolved to %#v which was not parsable by "+
				"Go's url parsing code. This is a likely configuration error in "+
				"twgit's config file, or possibly a bad value for the username was provided.",
			tw.URL.String(), resolvedStr)
	}

	return u, nil
}
