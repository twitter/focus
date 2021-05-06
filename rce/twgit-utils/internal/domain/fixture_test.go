package domain

import (
	"testing"

	"git.twitter.biz/focus/rce/twgit-utils/internal/testutils"
	"git.twitter.biz/focus/rce/twgit-utils/internal/validation"
	"github.com/go-playground/validator/v10"
)

type (
	Fixture struct {
		*testutils.Fixture
		V *validator.Validate
	}
)

func NewFixture(t *testing.T) *Fixture {
	return &Fixture{
		Fixture: testutils.NewFixture(t),
		V:       validation.NewValidator(),
	}
}

func (f *Fixture) NoError(err error, msgAndArgs ...interface{}) {
	if err == nil {
		return
	}

	// if the caller didn't provide an alternate message
	if len(msgAndArgs) == 0 {
		msgs := validation.SprintValidationErrors(err, validation.ValidationDebugErrTemplate)
		// and we were able to format the error
		if msgs != "" {
			// fail with the formatted errors
			f.FailNowf("validation failed", "err: %s\n%s", err, msgs)
		}
	}

	// otherwise just pass through
	f.Fixture.NoError(err, msgAndArgs...)
}

// it's pretty lame that this needs to be copy-pasta around :P
func (f *Fixture) UnmarshalConfig() (conf *Config) {
	conf, err := LoadConfigFromYaml(f.ReadProjectRootRelativeFile(f.ConfigPath))
	f.NoError(err)
	return conf
}
