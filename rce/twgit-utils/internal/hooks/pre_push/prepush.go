package pre_push

import (
	"bufio"
	"io"
	"regexp"
	"strings"

	"git.twitter.biz/focus/rce/twgit-utils/internal/domain"
	rslv "git.twitter.biz/focus/rce/twgit-utils/internal/resolver"
	"github.com/pkg/errors"

	"github.com/go-playground/validator/v10"
)

type (
	PrePushConfig struct {
		Resolver   rslv.Resolver
		Input      io.Reader
		Err        io.Writer
		RemoteName string
		RemoteDest string
	}

	RefSHA1 struct {
		Ref  string
		SHA1 string `v:"required,validSHA1"`
	}

	LocalRemoteRefs struct {
		Local  RefSHA1
		Remote RefSHA1
	}
)

func (p *PrePushConfig) IsTwgitUrl() bool {
	return strings.HasPrefix(p.RemoteDest, "twgit://")
}

func (p *PrePushConfig) TwgitURL() (url *domain.TwgitURL, err error) {
	if p.IsTwgitUrl() {
		return domain.ParseTwgitURL(p.RemoteDest)
	}
	return nil, errors.Errorf("RemoteDest was not a twgit url: %#v", p.RemoteDest)
}

var validSha1RE = regexp.MustCompile(`^[0-9a-f]{40}$`)

func validateSha1(fl validator.FieldLevel) bool {
	return validSha1RE.MatchString(fl.Field().String())
}

func validateDevPush(config *PrePushConfig, lrrs []LocalRemoteRefs) error {
	return nil
}

func readInput(in io.Reader) (lrr []LocalRemoteRefs, err error) {
	s := bufio.NewScanner(in)

	for s.Scan() {
		els := strings.Split(s.Text(), " ")
		if len(els) != 4 {
			return nil, errors.Errorf(
				"incorrect formatting of input line, "+
					"expected 4 elements separated by spaces: %#v", s.Text())
		}
		lrr = append(lrr, LocalRemoteRefs{
			Local:  RefSHA1{els[0], els[1]},
			Remote: RefSHA1{els[2], els[3]},
		})
	}

	return lrr, nil
}

func Run(config *PrePushConfig) error {
	lrr, err := readInput(config.Input)
	if err != nil {
		return err
	}
	_ = lrr
	return nil
}
