package domain

import (
	"fmt"
	"text/template"

	"git.twitter.biz/focus/rce/twgit-utils/internal/userhash"
	"git.twitter.biz/focus/rce/twgit-utils/internal/validation"
	"github.com/icza/dyno"
	"github.com/mitchellh/mapstructure"
	"github.com/pkg/errors"
	"gopkg.in/yaml.v3"
)

type (
	// Profile contains information necessary to route client requests to a git
	// backend node with a selected server-presented view of the available refs.
	// The view is important, because ref negotiation between client and server
	// is expensive, and is a major contributor to pushes and fetches being perceived
	// as slow. This information is used to resolve a TwgitURL into a connection
	// string that git supports natively by evaluating the Backend go template
	// string.
	Profile struct {
		// Host is the backend host portion of the git-native url
		Host string `reg:"host" v:"required,hostname_rfc1123"`
		// ReadonlyHost is an alternative host for doing pulls from.
		// This is currently not implemneted in the resolver, but there are plans
		// to add it.
		ReadonlyHost string `reg:"readonlyhost" v:"omitempty,hostname_rfc1123"`
		// The name of the repo. This will be filled in by the containing
		// Repo struct after loading.
		Name string `reg:"name" v:"required,printascii"`
		// There may be several backend URLs that provide different views of the
		// same repository. This name identifies the particular one we're configuring.
		// Some roles are:
		//
		// * main:    A master-only repository that is written to by CI and read by users.
		//            This is the authoritative source of the 'master' ref.
		// * dev:     A potentially sharded collection of backend nodes that each serve
		//            a portion of the "work in progress" refs to developers and provide
		//            (by default) a view of only a given user's refs when that user pushes
		//            or pulls from this profile.
		// * archive: A (potentially sharded) backend that contains the complete history. If a
		//            ref in the dev repo is expired, it can be moved to this repository.
		//            This is also where all tags are created and kept.
		// * ci:      A backend group that serves the EE infrastructure with its own specialized
		//            routing rulse.
		Role string `reg:"role" v:"required,oneof=main dev archive ci"`
		// Backend contains a golang text/template.Template string that will be evaluated
		// using the BackendVars struct.
		Backend string `reg:"backend" v:"required,contains=://"`
		// Nodes is a list of hostname fragments that will be selected from using a consistent
		// hashing algorithm based on the username.
		Nodes []string `reg:"nodes" v:"dive,hostname_rfc1123"`
		// Views contains a list of valid view names understood by the backend CGI and will be used
		// to selectively show and hide patterns of refs during the negotiation phase of a push or pull.
		Views []string `reg:"views" v:"unique,dive,oneof=~user _all _tags"`
		// DefaultView is used when the backend has several views defined, but the TwgitURL hasn't
		// specified which to use. For example, `twgit://source/dev` could specify `~user` here to
		// use the username hashing view by default.
		DefaultView string `reg:"defaultview" v:"omitempty,oneof=~user _all _tags"`
		// Scheme is the transport mechanism used to connect to the backend (almost always 'https'
		// but included for completeness)
		Scheme string `reg:"scheme" v:"required,oneof=http https"`
	}

	// Default allows users to define values that will be filled in if not specified on the
	// Profile struct for a particular Repo.
	Default struct {
		Host         string `reg:"host" v:"omitempty,hostname_rfc1123"`
		ReadonlyHost string `reg:"readonlyhost" v:"omitempty,hostname_rfc1123"`
		Scheme       string `reg:"scheme" v:"omitempty,oneof=http https"`
	}

	// Hasher defines the input parameters for the hash ring implementation we're using.
	Hasher struct {
		PartitionCount    int     `reg:"partitioncount" v:"required,min=1"`
		ReplicationFactor int     `reg:"replicationfactor" v:"required,min=1"`
		Load              float64 `reg:"load" v:"required,gt=0.0"`
	}

	Repo struct {
		// Name is the name we've given this bag of objects and refs. Git itself
		// doesn't have a concept of a repository having a particular name, so
		// we define that for it here. This along with the Role name is used
		// by the Resolver to choose what Profile instance to resolve a particular
		// request.
		Name     string    `reg:"name" v:"required,printascii"`
		Defaults Default   `reg:"defaults" v:"dive"`
		Profiles []Profile `reg:"profiles" v:"required,min=1,unique=Role,dive"`
	}

	// Config represents the serialized structure of the configuration file. It is used
	// to Unmarshal the data within, and provides convenience methods for searching for
	// particular records.
	Config struct {
		Version        int    `reg:"version" v:"required,eq=1"`
		Repos          []Repo `reg:"repos" v:"required,unique=Name,dive"`
		Hasher         Hasher `reg:"hasher" v:"dive"`
		profileRoleIdx map[string]map[string]*Profile
	}

	// BackendTemplateVars contains the available values for the URL templates.
	BackendTemplateVars struct {
		Scheme  string
		Host    string
		Repo    string
		Role    string
		Node    string
		View    string
		Options string
	}
)

const (
	AllView  = "_all"
	TagsView = "_tags"
	UserView = "~user"
)

func (p *Profile) templateName() string {
	return fmt.Sprintf("%s-%s-backend", p.Name, p.Role)
}

// Template returns a template.Template instance based on the parsed
// Backend string
func (p *Profile) Template() (t *template.Template, err error) {
	if t, err = template.New(p.templateName()).Parse(p.Backend); err != nil {
		return nil, errors.Wrapf(err, "failed to parse template %#v", p.templateName())
	}
	return t, nil
}

func (p *Profile) HasAnyViews() bool {
	return p.Views != nil && len(p.Views) > 0
}

// ViewOrDefault will return 'view' unless it's the empty string, in which case
// it will return the configured DefaultView.
func (p *Profile) ViewOrDefault(view string) string {
	if view != "" {
		return view
	}
	return p.DefaultView
}

func (p *Profile) HasView(s string) (b bool) {
	for i := range p.Views {
		if p.Views[i] == s {
			return true
		}
	}
	return false
}

func (c *Config) FindProfile(repo, role string) (p *Profile) {
	rmap, ok := c.profileRoleIdx[repo]
	if !ok {
		return nil
	}
	if p, ok = rmap[role]; ok {
		return p
	}
	return nil
}

type NotFoundError struct {
	Repo    string
	Role    string
	Message string
}

func (e *NotFoundError) Error() string {
	return fmt.Sprintf("%s repo=%#v, role=%#v", e.Message, e.Repo, e.Role)
}

var _ error = new(NotFoundError)

func (c *Config) Find(repo, role string) (p *Profile, err error) {
	p = c.FindProfile(repo, role)
	if p == nil {
		return nil, &NotFoundError{repo, role, "Could not find repo/role combination in config"}
	}

	return p, nil
}

func (c *Config) HasherConfig(nodes []string) (hc *userhash.HasherConfig, err error) {
	if nodes == nil {
		return nil, nil
	}

	return &userhash.HasherConfig{
		PartitionCount:    c.Hasher.PartitionCount,
		ReplicationFactor: c.Hasher.ReplicationFactor,
		Load:              c.Hasher.Load,
		Nodes:             nodes,
	}, nil
}

func loadConfigFromMap(m map[string]interface{}) (cfg *Config, err error) {
	if m, err = dyno.GetMapS(m, "twgit"); err != nil {
		return nil, errors.Wrap(err, "failed to get 'twgit' key from config")
	}

	var c Config

	var d *mapstructure.Decoder
	if d, err = mapstructure.NewDecoder(
		&mapstructure.DecoderConfig{
			DecodeHook: mapstructure.ComposeDecodeHookFunc(
				mapstructure.StringToTimeDurationHookFunc()),
			Metadata:         nil,
			Result:           &c,
			WeaklyTypedInput: true,
			TagName:          "reg",
		},
	); err != nil {
		return nil, errors.Wrap(err, "failed to create mapstructure.NewDecoder")
	}

	if err = d.Decode(m); err != nil {
		return nil, errors.Wrap(err, "mapstructure.Decode failed")
	}

	PostUnmarshal(&c)

	if err = validation.NewValidator().Struct(&c); err != nil {
		return nil, err
	}

	return &c, nil
}

func LoadConfigFromYaml(data []byte) (cfg *Config, err error) {
	m := make(map[string]interface{})

	if err = yaml.Unmarshal(data, m); err != nil {
		return nil, errors.Wrap(err, "failed to unmarshal config")
	}

	return loadConfigFromMap(m)
}

// generate the maps we use to resolve ropeo and role names to their appropriate
// config objects. This is pretty gross, changing internal state in this way,
// but it's only called from LoadConfig and the tests.
func PostUnmarshal(c *Config) {
	if c.profileRoleIdx == nil {
		c.profileRoleIdx = make(map[string]map[string]*Profile)
		for i := range c.Repos {
			name := c.Repos[i].Name
			defaults := c.Repos[i].Defaults

			for j := range c.Repos[i].Profiles {
				var rmap map[string]*Profile
				var ok bool

				// we make a reference to this profile
				// so that we can modify it instead of getting a copy
				// of it in 'p'
				p := &c.Repos[i].Profiles[j]

				p.Name = name
				if p.Host == "" {
					p.Host = defaults.Host
				}
				if p.ReadonlyHost == "" {
					p.ReadonlyHost = defaults.ReadonlyHost
				}
				if p.Scheme == "" {
					p.Scheme = defaults.Scheme
				}

				if rmap, ok = c.profileRoleIdx[p.Name]; !ok {
					rmap = make(map[string]*Profile)
				}

				rmap[p.Role] = p
				c.profileRoleIdx[p.Name] = rmap
			}
		}
	}
}
