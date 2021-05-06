package update

import (
	"fmt"
	"strconv"
	"strings"

	"git.twitter.biz/focus/rce/twgit-utils/internal/common"
	"git.twitter.biz/focus/rce/twgit-utils/internal/validation"
	"github.com/go-playground/validator/v10"
)

type (
	UpdateHookArgs struct {
		RefName                    string
		OldOid                     string
		NewOid                     string
		RemoteUser                 string `v:"required_with=CheckOwnerMatches"`
		CheckOwnerMatches          bool
		TagCreateOrUpdateForbidden bool
	}

	NotYourRefError struct {
		RefName    string
		RemoteUser string
		Owner      string
	}

	TagCreateOrUpdateForbiddenError struct {
		RefName string
	}
)

const ZeroSHA1 = "0000000000000000000000000000000000000000"
const UpdateHookGitConfigPrefix = "twgit.updatehook."

var vv *validator.Validate = validation.NewValidator()

func validate(uha *UpdateHookArgs) (err error) {
	return vv.Struct(uha)
}

func LoadUpdateHookArgsFromEnv(envVisitor common.KeyValueVisitor, uha *UpdateHookArgs) (err error) {
	err = envVisitor(func(k, v string) (ierr error) {
		switch k {
		case "REMOTE_USER":
			uha.RemoteUser = v
		case "TWGIT_CHECK_OWNER_MATCHES":
			uha.CheckOwnerMatches, ierr = strconv.ParseBool(v)
		case "TWGIT_TAG_CREATE_OR_UPDATE_FORBIDDEN":
			uha.TagCreateOrUpdateForbidden, ierr = strconv.ParseBool(v)
		default:
		}

		return ierr
	})

	if err != nil {
		return err
	}

	return validate(uha)
}

func LoadUpdateConfigFromGit(gitConfigVisitor common.KeyValueVisitor, uha *UpdateHookArgs) (err error) {
	if err = gitConfigVisitor(func(k, v string) (ierr error) {
		k = strings.ToLower(k) // normalize to lower case

		if strings.HasPrefix(k, UpdateHookGitConfigPrefix) {
			switch name := strings.TrimPrefix(k, UpdateHookGitConfigPrefix); name {
			case "checkownermatches":
				uha.CheckOwnerMatches, ierr = strconv.ParseBool(v)
			case "tagcreateorupdateforbidden":
				uha.TagCreateOrUpdateForbidden, ierr = strconv.ParseBool(v)
			default:
			}
		}

		return ierr
	}); err != nil {
		return err
	}

	return validate(uha)
}

func (e *NotYourRefError) Error() string {
	return fmt.Sprintf(
		"Remote user %#v tried to update ref %#v which may only be updated by user %#v",
		e.RemoteUser, e.RemoteUser, e.Owner,
	)
}

var _ error = (*NotYourRefError)(nil)

func (e *TagCreateOrUpdateForbiddenError) Error() string {
	return fmt.Sprintf(
		"Tags are not stored in this repo, rejecting %#v", e.RefName)
}

var _ error = (*TagCreateOrUpdateForbiddenError)(nil)

func (a *UpdateHookArgs) Stripped() string {
	if strings.HasPrefix(a.RefName, "refs/heads/") {
		return strings.TrimPrefix(a.RefName, "refs/heads/")
	}
	return ""
}

func (a *UpdateHookArgs) Owner() string {
	var stripped string
	if stripped = a.Stripped(); stripped == "" {
		return ""
	}
	if i := strings.Index(stripped, "/"); i > 0 {
		return stripped[0:i]
	}
	return ""
}

func (a *UpdateHookArgs) IsUpdate() bool {
	return a.NewOid != ZeroSHA1 && a.OldOid != ZeroSHA1
}

func (a *UpdateHookArgs) IsCreate() bool {
	return a.OldOid == ZeroSHA1
}

func (a *UpdateHookArgs) IsDelete() bool {
	return a.NewOid == ZeroSHA1
}

func checkOwnerMatches(a *UpdateHookArgs) error {
	if !a.CheckOwnerMatches {
		return nil
	}

	var owner string
	if owner = a.Owner(); owner != "" && owner != a.RemoteUser {
		return &NotYourRefError{a.RefName, a.RemoteUser, owner}
	}

	return nil
}

func tagsForbidden(a *UpdateHookArgs) error {
	if !a.TagCreateOrUpdateForbidden {
		return nil
	}
	if strings.HasPrefix(a.RefName, "refs/tags/") && (a.IsCreate() || a.IsUpdate()) {
		return &TagCreateOrUpdateForbiddenError{a.RefName}
	}
	return nil
}

func Run(a *UpdateHookArgs) (err error) {
	if err = checkOwnerMatches(a); err != nil {
		return err
	}
	if err = tagsForbidden(a); err != nil {
		return err
	}
	return nil
}
