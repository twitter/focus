package config

import (
	"bytes"
	"fmt"
	"strconv"
	"strings"
	"text/template"
	"time"

	"github.com/go-playground/validator/v10"
	"github.com/pkg/errors"
	log "github.com/sirupsen/logrus"

	"git.twitter.biz/focus/rce/twgit-utils/internal/common"
	"git.twitter.biz/focus/rce/twgit-utils/internal/git"
)

// This handles loading the git config out of the repository itself
// when the config is stored in a specially named ref. This ref can
// be hosted either in the same repo on the backend or in a different
// repo, because it's an orgphan branch and does not share history with
// the repository being managed.

type (
	RefConfig struct {
		// the name of the local ref that tracks the head of the admin config
		LocalRef string `reg:"localref" v:"required,startswith=refs/"`
		// The remote ref we'll fetch
		RemoteRef string `reg:"remoteref" v:"required,startswith=refs/"`
		// Either a URL describing the remote or the name of a proper git remote
		// that will be used to fetch the rmeote ref.
		Remote string `reg:"remotename" v:"required"`
		// The path relative to the admin ref that will read out of the repository
		// using git cat-file.
		BlobPath string `reg:"blobpath" v:"required"`
		// This code will check the remote ref every UpdateInterval to see if it's been
		// changed. We store the last update time in the git config for this repo.
		// If this value is not set, we will check for updates every 15m.
		UpdateInterval time.Duration `reg:"updateinterval" v:"required,min=0"`
		// The last time we checked for an update. Epoch time if we've never checked
		LastUpdateTimeUnix int64 `reg:"lastfetch"`
	}
)

const (
	DefaultUpdateInterval    time.Duration = 15 * time.Minute
	LastUpdateTimeUnixGitKey               = "twgit.admin.lastfetch"
	TwgitAdminPrefix                       = "twgit.admin."
	TwgitAdminEnvPrefix                    = "TWGIT_ADMIN_"
)

type (
	ReplaceFn func(s string) string
	FilterFn  func(s string) bool
)

var NoOpReplaceFn ReplaceFn = func(s string) string { return s }
var TakeAll FilterFn = func(s string) bool { return true }

func FilterOnPrefix(pfx string) FilterFn {
	return func(s string) bool {
		return strings.HasPrefix(s, pfx)
	}
}

func newValidator() *validator.Validate {
	v := validator.New()
	v.SetTagName("v")
	return v
}

var DefaultRefConfig = RefConfig{
	LocalRef:           "refs/admin/twgit",
	RemoteRef:          "refs/admin/twgit",
	Remote:             "origin",
	BlobPath:           "twgit.yaml",
	UpdateInterval:     DefaultUpdateInterval,
	LastUpdateTimeUnix: 0,
}

// unmarshals the field assigned to k from the value v. if k is not known, it's ignored.
func (rc *RefConfig) setFieldFromStrings(k, v string) (err error) {
	switch name := k; name {
	case "lastfetch":
		if rc.LastUpdateTimeUnix, err = strconv.ParseInt(v, 10, 64); err != nil {
			return errors.Errorf("failed to parse %#v as int64 for key %#v", v, name)
		}
	case "localref":
		rc.LocalRef = v
	case "remoteref":
		rc.RemoteRef = v
	case "remotename":
		rc.Remote = v
	case "updateinterval":
		if rc.UpdateInterval, err = time.ParseDuration(v); err != nil {
			return errors.Errorf("failed to parse %#v as a duration for key %#v", v, name)
		}
	case "blobpath":
		rc.BlobPath = v
	}
	return
}

func LoadRefConfigFromGit(gitConfigVisitor common.KeyValueVisitor, rc *RefConfig) (err error) {
	if err = gitConfigVisitor(func(k, v string) error {
		if !strings.HasPrefix(k, TwgitAdminPrefix) {
			return nil
		} else {
			return rc.setFieldFromStrings(
				strings.TrimPrefix(k, TwgitAdminPrefix), v)
		}
	}); err != nil {
		return err
	}

	if err = newValidator().Struct(rc); err != nil {
		return errors.Wrap(err, "validation failed")
	}

	return nil
}

func LoadRefConfigFromEnv(envVisitor common.KeyValueVisitor, rc *RefConfig) (err error) {
	if err = envVisitor(func(k, v string) error {
		if !strings.HasPrefix(k, TwgitAdminEnvPrefix) {
			return nil
		}

		return rc.setFieldFromStrings(
			strings.ToLower(strings.TrimPrefix(k, TwgitAdminEnvPrefix)), v)
	}); err != nil {
		return err
	}

	if err = newValidator().Struct(rc); err != nil {
		return errors.Wrap(err, "validation failed")
	}

	return nil
}

var debugTemplate = template.Must(
	template.New("RefConfigString").Parse(`
RefConfig {
	LocalRef:           "{{.LocalRef}}",
	RemoteRef:          "{{.RemoteRef}}",
	Remote:             "{{.Remote}}",
	BlobPath:           "{{.BlobPath}}",
	UpdateInterval:     "{{.UpdateInterval}}",
	LastUpdateTimeUnix: {{.LastUpdateTimeUnix}},
}
`),
)

func (rc *RefConfig) String() string {
	var buf bytes.Buffer
	err := debugTemplate.Execute(&buf, rc)
	if err != nil {
		log.Fatalf("could not execute RefConfig.String template: %#v", err)
	}
	return buf.String()
}

func (rc *RefConfig) BlobConfig(repo *git.Repo) (bc *BlobConfig) {
	return NewBlobConfig(rc, repo)
}

type (
	// BlobConfig (besides being a bad name) is what uses the RefConfig to
	// update, read, and interact with the git repo.
	BlobConfig struct {
		*RefConfig
		*git.Repo
	}
)

func NewBlobConfig(rc *RefConfig, repo *git.Repo) (cu *BlobConfig) {
	return &BlobConfig{rc, repo}
}

func (c *BlobConfig) LastUpdateTime() time.Time {
	return time.Unix(int64(c.LastUpdateTimeUnix), 0)
}

// returns the UpdateInterval but subs the default 15m duration if the value
// is not set
func (c *BlobConfig) interval() time.Duration {

	if c.UpdateInterval == time.Duration(0) {
		return DefaultUpdateInterval
	}
	return c.UpdateInterval
}

func (c *BlobConfig) updateLastFetch(t time.Time) (err error) {
	_, err = c.Run(
		"config", "--local", "--type=int", LastUpdateTimeUnixGitKey, fmt.Sprintf("%d", t.UTC().Unix()),
	)
	return err
}

var Epoch = time.Unix(0, 0).UTC()

func (c *BlobConfig) outdated() bool {
	now := time.Now().UTC().Unix()
	last := c.LastUpdateTimeUnix

	if int64(last) > now {
		log.Warn("admin ref lastupdatetime was in the future! invalidating and fetching")
		c.updateLastFetch(Epoch)
		return true
	}

	diff := now - int64(last)
	interval := int64(c.interval().Seconds())

	log.Debugf(
		"now: %#v, last: %#v, diff: %#v, interval sec: %#v",
		now, last, diff, interval,
	)

	return diff > interval
}

const (
	UpdateCreatedRef common.UpdateStatus = "created local branch"
	UpdateUpdatedRef common.UpdateStatus = "updated existing local branch"
	UpdateNotTimeYet common.UpdateStatus = "branch TTL not exceeded yet"
	UpdateNoChange   common.UpdateStatus = "local branch up to date with remote"
	UpdateError      common.UpdateStatus = ""
)

// Update the admin ref from the remote using the configured values.
// If we did update the admin ref, return true.
func (c *BlobConfig) Update() (common.UpdateStatus, error)      { return c.update(false) }
func (c *BlobConfig) ForceUpdate() (common.UpdateStatus, error) { return c.update(true) }

func (c *BlobConfig) update(force bool) (updated common.UpdateStatus, err error) {
	sha1, err := c.localExists()
	if err != nil {
		return UpdateError, err
	}

	log.Debugf("%s %s", sha1, c.LocalRef)

	outdated := c.outdated()

	log.Debug(c.String())

	if force || sha1 == "" || outdated {
		if err = c.doFetch(); err != nil {
			return UpdateError, err
		}
	}

	if !outdated && !force {
		return UpdateNotTimeYet, nil
	}

	curSha1, err := c.localExists()
	if err != nil {
		return UpdateError, err
	}

	log.Debugf("orig: %s cur: %s, orig != cur: %#v", sha1, curSha1, sha1 != curSha1)

	if sha1 == "" {
		return UpdateCreatedRef, nil
	} else if curSha1 == sha1 {
		return UpdateNoChange, nil
	}

	return UpdateUpdatedRef, nil
}

func (c *BlobConfig) localExists() (sha1 string, err error) {
	var cr git.CommandResult

	if cr, err = c.Run("show-ref", "--", c.LocalRef); err != nil {
		if cef, ok := err.(*git.CommandFailedError); ok {
			if cef.ExitCode == 1 && cef.Stderr == "" {
				return "", nil
			}
		} else {
			return "", errors.Wrap(err, "show-ref failed")
		}
	}

	return cr.OutputLines()[0][0:40], nil
}

func (c *BlobConfig) doFetch() (err error) {
	// need to add a version check, no-auto-gc isn't in 2.20
	args := []string{"fetch", "--no-tags", "--refmap=", "--quiet" /*"--no-auto-gc", */}
	exists, err := c.localExists()
	if err != nil {
		return err
	}

	if exists != "" {
		args = append(args, fmt.Sprintf("--negotiation-tip=%s", c.LocalRef))
	}

	args = append(args, c.Remote, fmt.Sprintf("+%s:%s", c.RemoteRef, c.LocalRef))

	cr, err := c.Run(args...)
	if err != nil {
		return err
	}
	if cr.ExitCode() != 0 {
		return errors.Errorf(
			"fetch command %#v failed exit code: %d",
			cr.Command().String(), cr.ExitCode())
	}

	return c.updateLastFetch(time.Now().UTC())
}

// ReadAll returns the contents of the configured blob at the configured ref
// as a byte slice
func (c *BlobConfig) ReadAll() (data []byte, err error) {
	return c.ReadPath(c.BlobPath)
}

// ReadString returns the contents of the configured blob at the configured ref
// as a string.
func (c *BlobConfig) ReadString() (s string, err error) {
	data, err := c.ReadAll()
	if err != nil {
		return "", err
	}
	return string(data), err
}

// ReadPath returns the contents
func (c *BlobConfig) ReadPath(path string) (data []byte, err error) {
	return c.Repo.CatFileBlob(c.LocalRef, path)
}

// shadows the one in RefConfig
// explodes, because you really shouldn't call this again
func (c *BlobConfig) BlobConfig(repo *git.Repo) (bc *BlobConfig) {
	panic("[BUG] called BlobConfig on BlobConfig instance")
}
