package domain

import (
	"fmt"
	"regexp"
	"strings"
	"testing"

	"git.twitter.biz/focus/rce/twgit-utils/internal/validation"
)

type (
	/* these allow for semi-declarative testing (see below) */
	pmod    func(*Profile)
	pmodPat struct {
		mod pmod
		re  string
	}
)

func (f *Fixture) newProfile() *Profile {
	return &Profile{
		Host:         "host.example.com",
		ReadonlyHost: "readonlyhost.example.com",
		Name:         "repo",
		Role:         "dev",
		Backend:      "{{.Scheme}}://{{.Host}}/{{.Role}}//{{.View}}",
		Nodes:        []string{"node00", "node01", "node02"},
		Views:        []string{"~user", "_all", "_tags"},
		DefaultView:  "~user",
		Scheme:       "https",
	}
}

func TestProfileValidations(t *testing.T) {
	f := NewFixture(t)
	defer f.Close()

	test := func(p *Profile, re string) {
		err := f.V.Struct(p)
		f.Error(err)
		f.Regexp(regexp.MustCompile(re), err.Error())
	}

	noErr := func(p *Profile) {
		if err := f.V.Struct(p); err != nil {
			errs := validation.FormatValidationErrors(err, nil)
			if errs != nil {
				f.NoError(err, strings.Join(errs, "\n")+"\n")
			} else {
				f.NoError(err)
			}
		}
	}

	// test base case
	noErr(f.newProfile())

	var p *Profile

	mkFail := func(f pmod, field, tag string) *pmodPat {
		return &pmodPat{f, fmt.Sprintf(`Profile\.%s.*'%s'`, field, tag)}
	}

	shouldFail := []*pmodPat{
		/* a func that will mutate p, the field name, and the validation name that failed */
		mkFail(func(p *Profile) { p.Host = "\n\tINVALID HOST\n\t" }, "Host", "hostname_rfc1123"),
		mkFail(func(p *Profile) { p.Host = "" }, "Host", "required"),
		mkFail(func(p *Profile) { p.ReadonlyHost = "\n\tINVALID HOST\n\t" }, "ReadonlyHost", "hostname_rfc1123"),
		mkFail(func(p *Profile) { p.Name = "" }, "Name", "required"),
		mkFail(func(p *Profile) { p.Role = "wtf" }, "Role", "oneof"),
		mkFail(func(p *Profile) { p.Backend = "ssh@foo" }, "Backend", "contains"),
		mkFail(func(p *Profile) { p.Backend = "" }, "Backend", "required"),
		mkFail(func(p *Profile) { p.Nodes = []string{"foo", "bar", "LOL OMG"} }, "Nodes", "hostname_rfc1123"),
		mkFail(func(p *Profile) { p.Views = []string{"~user", "what"} }, "Views", "oneof"),
		mkFail(func(p *Profile) { p.Scheme = "" }, "Scheme", "required"),
	}

	for _, pp := range shouldFail {
		p = f.newProfile()
		pp.mod(p)
		test(p, pp.re)
	}

	validRoles := []string{"main", "dev", "archive", "ci"}
	for i := range validRoles {
		p = f.newProfile()
		p.Role = validRoles[i]
		f.NoError(f.V.Struct(p))
	}

	p = f.newProfile()
	p.DefaultView = ""
	f.NoError(f.V.Struct(p))

	p = f.newProfile()
	p.Views = []string{"~user", "_all", "_tags"}
	f.NoError(f.V.Struct(p))

	validSchemes := []string{"http", "https"}
	for i := range validSchemes {
		p = f.newProfile()
		p.Scheme = validSchemes[i]
		f.NoError(f.V.Struct(p))
	}
}

type (
	dmod    func(d *Default)
	dmodPat struct {
		mod dmod
		re  string
	}
)

func (f *Fixture) newDefault() *Default {
	return &Default{
		Host:         "host",
		ReadonlyHost: "rohost",
		Scheme:       "https",
	}
}

func TestDefaultValidations(t *testing.T) {
	f := NewFixture(t)
	defer f.Close()

	test := func(d *Default, re string) {
		err := f.V.Struct(d)
		f.Error(err)
		f.Regexp(regexp.MustCompile(re), err.Error())
	}

	noErr := func(d *Default) { f.NoError(f.V.Struct(d)) }

	// make sure the default case works
	noErr(f.newDefault())

	// these modify a Domain struct in ways that should fail
	// validation that match the regep pattern givn in re
	shouldFail := []dmodPat{
		{func(d *Default) { d.Host = "INVALID HOST" }, `Default\.Host.*hostname_rfc1123`},
		{func(d *Default) { d.ReadonlyHost = "INVALID HOST" }, `Default\.ReadonlyHost.*hostname_rfc1123`},
		{func(d *Default) { d.Scheme = "ssh" }, `Default\.Scheme.*oneof`},
	}

	for _, sf := range shouldFail {
		d := f.newDefault()
		sf.mod(d)
		test(d, sf.re)
	}

	// these modify a Domain obj in ways that should pass validation
	shouldPass := []dmod{
		func(d *Default) { d.Host = "" },
		func(d *Default) { d.Host = "valid-host" },
		func(d *Default) { d.ReadonlyHost = "" },
		func(d *Default) { d.ReadonlyHost = "valid-host.domain" },
		func(d *Default) { d.Scheme = "" },
		func(d *Default) { d.Scheme = "http" },
		func(d *Default) { d.Scheme = "https" },
	}

	for _, sp := range shouldPass {
		d := f.newDefault()
		sp(d)
		noErr(d)
	}
}

type (
	/* these allow for semi-declarative testing (see below) */
	hmod    func(d *Hasher)
	hmodPat struct {
		mod hmod
		re  string
	}
)

func (f *Fixture) newHasher() *Hasher {
	return &Hasher{
		PartitionCount:    23,
		ReplicationFactor: 42,
		Load:              1.87,
	}
}

func TestHasherValidation(t *testing.T) {
	f := NewFixture(t)
	defer f.Close()

	test := func(h *Hasher, re string) {
		err := f.V.Struct(h)
		f.Error(err)
		f.Regexp(regexp.MustCompile(re), err.Error())
	}

	noErr := func(h *Hasher) { f.NoError(f.V.Struct(h)) }

	// make sure the default case works
	noErr(f.newHasher())

	shouldFail := []hmodPat{
		{func(h *Hasher) { h.PartitionCount = -1 }, `Hasher\.PartitionCount.*'min'`},
		{func(h *Hasher) { h.PartitionCount = 0 }, `Hasher\.PartitionCount.*'required'`},
		{func(h *Hasher) { h.ReplicationFactor = -1 }, `Hasher\.ReplicationFactor.*'min'`},
		{func(h *Hasher) { h.ReplicationFactor = 0 }, `Hasher\.ReplicationFactor.*'required'`},
		{func(h *Hasher) { h.Load = -1.0 }, `Hasher\.Load.*'gt'`},
		{func(h *Hasher) { h.Load = 0.0 }, `Hasher\.Load.*'required'`},
	}

	for _, sf := range shouldFail {
		h := f.newHasher()
		sf.mod(h)
		test(h, sf.re)
	}
}

type (
	/* these allow for semi-declarative testing (see below) */
	rmod    func(r *Repo)
	rmodPat struct {
		mod rmod
		re  string
	}
)

func (f *Fixture) newRepo() *Repo {
	return &Repo{
		Name:     "repo",
		Defaults: *f.newDefault(),
		Profiles: []Profile{*f.newProfile()},
	}
}

func TestRepoValidation(t *testing.T) {
	f := NewFixture(t)
	defer f.Close()

	test := func(r *Repo, re string) {
		err := f.V.Struct(r)
		f.Error(err)
		f.Regexp(regexp.MustCompile(re), err.Error())
	}

	noErr := func(r *Repo) { f.NoError(f.V.Struct(r)) }

	noErr(f.newRepo())

	shouldFail := []rmodPat{
		{func(r *Repo) { r.Name = "NOT\tVALID" }, `Repo\.Name.*'printascii'`},
		{func(r *Repo) { r.Name = "" }, `Repo\.Name.*'required'`},
		/* this tests the 'dive' tag */
		{func(r *Repo) { r.Defaults.Scheme = "ssh" }, `Repo\.Defaults\.Scheme.*'oneof'`},
		/* this tests the 'dive' tag */
		{func(r *Repo) { r.Profiles[0].Role = "roll" }, `Repo\.Profiles\[0\]\.Role.*'oneof'`},
		{func(r *Repo) { r.Profiles = nil }, `Repo\.Profiles.*'required'`},
		{func(r *Repo) { r.Profiles = []Profile{} }, `Repo\.Profiles.*'min'`},
		{
			func(r *Repo) {
				r.Profiles = append(r.Profiles, r.Profiles[0])
			},
			`Repo\.Profiles.*'unique'`,
		},
	}

	for _, sf := range shouldFail {
		r := f.newRepo()
		sf.mod(r)
		test(r, sf.re)
	}

	shouldPass := []rmod{
		func(d *Repo) { d.Defaults = *new(Default) },
	}

	for _, sp := range shouldPass {
		r := f.newRepo()
		sp(r)
		noErr(r)
	}
}

func (f *Fixture) newConfig() *Config {
	c := &Config{
		Version: 1,
		Repos:   []Repo{*f.newRepo()},
		Hasher:  *f.newHasher(),
	}
	PostUnmarshal(c)
	return c
}

type (
	cmod    func(c *Config)
	cmodPat struct {
		mod cmod
		re  string
	}
)

func TestConfigValidation(t *testing.T) {
	f := NewFixture(t)
	defer f.Close()

	test := func(c *Config, re string) {
		err := f.V.Struct(c)
		f.Error(err)
		f.Regexp(regexp.MustCompile(re), err.Error())
	}

	noErr := func(c *Config) { f.NoError(f.V.Struct(c)) }

	noErr(f.newConfig())

	shouldFail := []cmodPat{
		{func(c *Config) { c.Version = 3 }, `Config\.Version.*'eq'`},
		{func(c *Config) { c.Version = 0 }, `Config\.Version.*'required'`},
		{func(c *Config) { c.Hasher.PartitionCount = -2 }, `Config\.Hasher.*'min'`},
		{func(c *Config) { c.Repos = nil }, `Config\.Repos.*'required'`},
		{func(c *Config) { c.Repos[0].Name = "\t" }, `Config\.Repos\[0\]\.Name.*'printascii'`},
		{
			func(c *Config) {
				c.Repos = append(c.Repos, c.Repos[0])
			},
			`Config\.Repos.*'unique'`,
		},
	}

	for _, sf := range shouldFail {
		c := f.newConfig()
		sf.mod(c)
		test(c, sf.re)
	}
}
