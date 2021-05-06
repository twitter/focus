package config

import (
	"os"
	"regexp"
	"strconv"
	"testing"
	"time"

	"git.twitter.biz/focus/rce/twgit-utils/internal/common"
)

func TestLoadRefConfigFromGit(t *testing.T) {
	f := NewFixture(t)
	defer f.Close()

	mv := common.NewMapVisitor(map[string]string{
		"twgit.admin.lastfetch":      "1969",
		"twgit.admin.localref":       "refs/admin/twgit-local",
		"twgit.admin.remoteref":      "refs/admin/twgit-remote",
		"twgit.admin.remotename":     "control",
		"twgit.admin.updateinterval": "30m25s",
		"twgit.admin.blobpath":       "fart.yaml",
	})

	rc := &RefConfig{}
	f.NoError(LoadRefConfigFromGit(mv, rc))

	f.Equal(
		&RefConfig{
			LocalRef:           "refs/admin/twgit-local",
			RemoteRef:          "refs/admin/twgit-remote",
			Remote:             "control",
			BlobPath:           "fart.yaml",
			UpdateInterval:     time.Duration(30*time.Minute + 25*time.Second),
			LastUpdateTimeUnix: 1969,
		},
		rc,
	)
}

func TestLoadRefConfigFromEnv(t *testing.T) {
	f := NewFixture(t)
	defer f.Close()

	mv := common.NewMapVisitor(map[string]string{
		"TWGIT_ADMIN_LASTFETCH":      "1969",
		"TWGIT_ADMIN_LOCALREF":       "refs/admin/twgit-local",
		"TWGIT_ADMIN_REMOTEREF":      "refs/admin/twgit-remote",
		"TWGIT_ADMIN_REMOTENAME":     "control",
		"TWGIT_ADMIN_UPDATEINTERVAL": "30m25s",
		"TWGIT_ADMIN_BLOBPATH":       "fart.yaml",
	})

	rc := &RefConfig{}
	f.NoError(LoadRefConfigFromEnv(mv, rc))

	f.Equal(
		&RefConfig{
			LocalRef:           "refs/admin/twgit-local",
			RemoteRef:          "refs/admin/twgit-remote",
			Remote:             "control",
			BlobPath:           "fart.yaml",
			UpdateInterval:     time.Duration(30*time.Minute + 25*time.Second),
			LastUpdateTimeUnix: 1969,
		},
		rc,
	)
}

func TestRefConfigStringer(t *testing.T) {
	f := NewFixture(t)
	defer f.Close()

	rc := &RefConfig{
		LocalRef:           "refs/admin/twgit-local",
		RemoteRef:          "refs/admin/twgit-remote",
		Remote:             "origin",
		BlobPath:           "twgit.yaml",
		UpdateInterval:     time.Duration(15 * time.Minute),
		LastUpdateTimeUnix: 1618706619,
	}
	f.Equal(`
RefConfig {
	LocalRef:           "refs/admin/twgit-local",
	RemoteRef:          "refs/admin/twgit-remote",
	Remote:             "origin",
	BlobPath:           "twgit.yaml",
	UpdateInterval:     "15m0s",
	LastUpdateTimeUnix: 1618706619,
}
`,
		rc.String(),
	)
}

func getLastfetch(f *Fixture) (unix int64, ok bool) {
	sint, ok, err := f.TestRepo.Config().Local().Get("twgit.admin.lastfetch")
	f.NoError(err)
	if !ok {
		return -1, ok
	}
	unixSec, err := strconv.ParseInt(sint, 10, 64)
	f.NoError(err)
	return unixSec, true
}

func loadAndReturnRefConfig(f *Fixture) *RefConfig {
	rc := DefaultRefConfig

	f.NoError(LoadRefConfigFromGit(f.TestRepo.Config().Visit, &rc))
	f.NoError(LoadRefConfigFromEnv(common.NewEnvVisitor(os.Environ()), &rc))

	return &rc
}

func TestBlobConfig(t *testing.T) {
	f := NewFixture(t).SetupGitTestRepos()
	defer f.Close()
	rc := loadAndReturnRefConfig(f)

	_, ok := getLastfetch(f)
	f.False(ok)

	updateStatus, err := NewBlobConfig(rc, f.TestRepo.Repo).Update()
	f.NoError(err)
	f.Equal(UpdateCreatedRef, updateStatus)
	f.RefMustExist(f.TestRepo.Repo, "refs/admin/twgit")

	// this value should be updated
	unixSec, ok := getLastfetch(f)
	f.True(ok)

	now := time.Now().UTC()
	f.InDelta(now.Unix(), unixSec, 1, "expected lastfetch config timestamp to be within 1s of now")

	rc = loadAndReturnRefConfig(f)

	// wait unitl the next wall-clock second, that way we'd know the fetch had updated
	// we could store the value with more precision, but that seems excessive
	nextSec := now.Add(1 * time.Second).Truncate(time.Second)
	time.Sleep(nextSec.Sub(now))

	// the second update call should not update the timestamp
	updateStatus, err = NewBlobConfig(rc, f.TestRepo.Repo).Update()
	f.NoError(err)
	f.Equal(UpdateNotTimeYet, updateStatus)
	newUnixSec, ok := getLastfetch(f)
	f.True(ok)
	f.Equal(unixSec, newUnixSec)

	// if tweak the last fetched to be epoch the next status should be "no change"
	f.NoError(f.TestRepo.
		Config().Local().Type("int").Set("twgit.admin.lastfetch", "0"))

	// refresh the ref config so we see the change in the timestamp
	rc = loadAndReturnRefConfig(f)

	updateStatus, err = NewBlobConfig(rc, f.TestRepo.Repo).Update()
	f.NoError(err)
	f.Equal(UpdateNoChange, updateStatus)

	expected := string(f.ReadProjectRootRelativeFile("config", "twgit.yaml"))
	bc := rc.BlobConfig(f.TestRepo.Repo)
	conf, err := bc.ReadString()
	f.NoError(err)
	f.Equal(expected, conf)
}

func TestParseErrors(t *testing.T) {
	f := NewFixture(t).SetupGitTestRepos()
	defer f.Close()

	err := LoadRefConfigFromGit(
		common.NewPairsVisitor("twgit.admin.updateinterval", "A HUNDRED MILLION YEARS"),
		&RefConfig{},
	)

	f.Error(err)
	f.Regexp(
		regexp.MustCompile(`^failed to parse .* as a duration for key "updateinterval"$`),
		err.Error())

	err = LoadRefConfigFromGit(
		common.NewPairsVisitor("twgit.admin.lastfetch", "last tuesday"),
		&RefConfig{},
	)

	f.Error(err)
	f.Regexp(
		regexp.MustCompile(`^failed to parse .* as int64 for key "lastfetch"$`),
		err.Error())
}
