package backend

import (
	"io"
	"regexp"
	"strings"

	log "github.com/sirupsen/logrus"

	"git.twitter.biz/focus/rce/twgit-utils/internal/common"
	"git.twitter.biz/focus/rce/twgit-utils/internal/validation"
	"github.com/go-playground/validator/v10"
	"github.com/pkg/errors"
)

type (
	Config struct {
		Env    []string
		Stdin  io.Reader
		Stdout io.Writer
		Stderr io.Writer
	}

	// Values that are set in the environment by the cgi host
	CGIEnv struct {
		GitBin         string `v:"required"`
		PathInfo       string `v:"required"`
		PathTranslated string `v:"required"`
		RequestURI     string `v:"required"`

		AdditionalHideRefs []string
	}
)

const (
	AppendHideRefsEnvKey = "TWGIT_BACKEND_APPEND_HIDE_REFS"
	// try to load defaults or values from the repo itself
	// (this is an opt-in feature)
	BackendLoadGitConfigEnvKey = "TWGIT_BACKEND_READ_GIT_CONFIG"

	DefaultGitBin = "/usr/bin/git"

	PathInfoKey       = "PATH_INFO"
	GitBinKey         = "GIT_BIN"
	PathTranslatedKey = "PATH_TRANSLATED"
	RequestURIKey     = "REQUEST_URI"
)

var (
	validHideRefsRe = regexp.MustCompile(`^[!]?refs(/|$)`)
	DefaultHideRefs = []string{
		"refs/heads",
		"refs/tags",
		"!refs/heads/master",
		"!refs/admin",
	}
)

func validHideRefsValue(s string) bool {
	return validHideRefsRe.MatchString(s)
}

func visitEnv(env []string, f common.KeyValueVisitorCb) (err error) {
	for _, kv := range env {
		q := strings.Index(kv, "=")
		if q < 0 {
			continue
		}
		k := kv[:q]
		v := kv[q+1:]

		if err = f(k, v); err != nil {
			return err
		}
	}
	return nil
}

func fieldNameToEnvVar(s string) string {
	switch s {
	case "PathInfo":
		return PathInfoKey
	case "PathTranslated":
		return PathTranslatedKey
	case "RequestURI":
		return RequestURIKey
	default:
		return ""
	}
}

// Return a CGIEnv instance containing values loaded from the environemnt.
// If any values are unset, returns an error describing the missing fields.
//
// Looks for a list of refs to hide from the environment key TWGIT_BACKEND_HIDE_DEFAULT. This
// can be used to override entries in the git config because later entries take
// precedence over earlier ones. The format is a comma separated list of
// entries like "refs/heads/foo,!refs/heads/foo/bar".
//
func NewCGIEnv(env []string) (cgie *CGIEnv, err error) {
	cgie = &CGIEnv{GitBin: DefaultGitBin}

	_ = visitEnv(env, func(k, v string) error {
		switch k {
		case PathInfoKey:
			cgie.PathInfo = v
		case PathTranslatedKey:
			cgie.PathTranslated = v
		case RequestURIKey:
			cgie.RequestURI = v
		case GitBinKey:
			cgie.GitBin = v
		case AppendHideRefsEnvKey:
			for _, hr := range strings.Split(v, ",") {
				if validHideRefsValue(hr) {
					cgie.AdditionalHideRefs = append(cgie.AdditionalHideRefs, hr)
				} else {
					log.Warnf(
						"env var %#v contained an invalid transfer.hideRefs value: %#v",
						AppendHideRefsEnvKey, hr,
					)
				}
			}
		}
		return nil
	})

	v := validation.NewValidator()
	if err = v.Struct(cgie); err != nil {
		// if we can, report back the actual env vars that were supposed to be set in the env
		// but weren't to make it easier for someone to fix the problem
		if errs, ok := err.(validator.ValidationErrors); ok {
			var missing []string
			for _, e := range errs {
				if envname := fieldNameToEnvVar(e.StructField()); envname != "" && e.Tag() == "required" {
					missing = append(missing, envname)
				}
			}
			if len(missing) > 0 {
				return nil, errors.Errorf(
					"The following env vars were expected to be set but were not: %s",
					strings.Join(missing, ","),
				)
			}
		}

		return nil, err
	}

	return cgie, nil
}
