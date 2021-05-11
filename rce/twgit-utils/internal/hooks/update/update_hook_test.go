package update

import (
	"testing"

	"git.twitter.biz/focus/rce/twgit-utils/internal/common"
	"git.twitter.biz/focus/rce/twgit-utils/internal/testutils"
)

type (
	Fixture struct {
		*testutils.Fixture
	}
)

func NewFixture(t *testing.T) *Fixture {
	return &Fixture{testutils.NewFixture(t)}
}

const nonZeroSHA1 = "0123456789012345678901234567890123456789"

func TestOwnerMatches(t *testing.T) {
	f := NewFixture(t)
	defer f.Close()

	f.NoError(Run(&UpdateHookArgs{
		RefName:           "refs/heads/alice/feature",
		RemoteUser:        "alice",
		OldOid:            ZeroSHA1,
		NewOid:            nonZeroSHA1,
		CheckOwnerMatches: true,
	}))

	e := Run(&UpdateHookArgs{
		RefName:           "refs/heads/alice/feature",
		RemoteUser:        "bob",
		OldOid:            ZeroSHA1,
		NewOid:            nonZeroSHA1,
		CheckOwnerMatches: true,
	})

	f.NotNil(e)
	err, ok := e.(*NotYourRefError)
	f.True(ok)
	f.Equal("alice", err.Owner)
	f.Equal("bob", err.RemoteUser)
	f.Equal("refs/heads/alice/feature", err.RefName)
}

func TestTagsForbidden(t *testing.T) {
	f := NewFixture(t)
	defer f.Close()

	// no-op not a tag
	f.NoError(Run(&UpdateHookArgs{
		RefName:                    "refs/heads/alice/feature",
		RemoteUser:                 "alice",
		OldOid:                     ZeroSHA1,
		NewOid:                     nonZeroSHA1,
		TagCreateOrUpdateForbidden: true,
	}))

	var err error

	// creation
	err = Run(&UpdateHookArgs{
		RefName:                    "refs/tags/alice/feature",
		RemoteUser:                 "alice",
		OldOid:                     ZeroSHA1,
		NewOid:                     nonZeroSHA1,
		TagCreateOrUpdateForbidden: true,
	})
	f.NotNil(err)

	f.Equal("Tags are not stored in this repo, rejecting \"refs/tags/alice/feature\"", err.Error())

	// update
	err = Run(&UpdateHookArgs{
		RefName:                    "refs/tags/alice/feature",
		RemoteUser:                 "alice",
		OldOid:                     nonZeroSHA1[0:39] + "8",
		NewOid:                     nonZeroSHA1,
		TagCreateOrUpdateForbidden: true,
	})
	f.NotNil(err)
	f.Equal("Tags are not stored in this repo, rejecting \"refs/tags/alice/feature\"", err.Error())

	// delete, allowed but ???
	err = Run(&UpdateHookArgs{
		RefName:                    "refs/tags/alice/feature",
		RemoteUser:                 "alice",
		OldOid:                     nonZeroSHA1,
		NewOid:                     ZeroSHA1,
		TagCreateOrUpdateForbidden: true,
	})
	f.Nil(err)
}

func TestUpdateHookArgsUtilityMethods(t *testing.T) {
	f := NewFixture(t)
	defer f.Close()

	uha := &UpdateHookArgs{
		RefName: "refs/heads/abc/xyz",
		OldOid:  ZeroSHA1,
		NewOid:  nonZeroSHA1,
	}

	f.True(uha.IsCreate())
	f.Equal("abc/xyz", uha.Stripped())
	f.Equal("abc", uha.Owner())

	uha.OldOid = nonZeroSHA1
	uha.NewOid = ZeroSHA1
	f.True(uha.IsDelete())

	uha.NewOid = nonZeroSHA1
	f.True(uha.IsUpdate())
}

func TestLoadUpdateHookArgsFromEnv(t *testing.T) {
	f := NewFixture(t)
	defer f.Close()

	uha := new(UpdateHookArgs)
	err := LoadUpdateHookArgsFromEnv(common.NewPairsVisitor(
		"REMOTE_USER", "jlennon",
		"TWGIT_CHECK_OWNER_MATCHES", "1",
		"TWGIT_TAG_CREATE_OR_UPDATE_FORBIDDEN", "true",
	), uha)
	f.NoError(err)

	f.Equal(
		&UpdateHookArgs{
			RefName:                    "",
			OldOid:                     "",
			NewOid:                     "",
			RemoteUser:                 "jlennon",
			CheckOwnerMatches:          true,
			TagCreateOrUpdateForbidden: true,
		},
		uha,
	)

	err = LoadUpdateHookArgsFromEnv(common.NewPairsVisitor(
		"REMOTE_USER", "jlennon",
		"TWGIT_CHECK_OWNER_MATCHES", "notabool",
		"TWGIT_TAG_CREATE_OR_UPDATE_FORBIDDEN", "true",
	), uha)
	f.Error(err)
}

func TestLoadUpdateHookArgsFromGit(t *testing.T) {
	f := NewFixture(t)
	defer f.Close()

	uha := new(UpdateHookArgs)
	uha.RemoteUser = "jlennon"

	err := LoadUpdateConfigFromGit(common.NewPairsVisitor(
		"twgit.updateHook.checkOwnerMatches", "1",
		"twgit.updateHook.tagCreateOrUpdateForbidden", "true",
	), uha)
	f.NoError(err)

	f.Equal(
		&UpdateHookArgs{
			RefName:                    "",
			OldOid:                     "",
			NewOid:                     "",
			RemoteUser:                 "jlennon",
			CheckOwnerMatches:          true,
			TagCreateOrUpdateForbidden: true,
		},
		uha,
	)
}
