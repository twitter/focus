package domain

import (
	"net/url"
	"strings"

	"github.com/pkg/errors"
)

// TwgitURL is a wrapper around a url.URL that presents the components of a twgit://
// url at an abstract level. One can think of a twgit url as a tuple written in a format
// that tricks git into calling our code to route the request. The tuple contains 4
// main elements:
//
//     twgit://repo/role/view?option=val
//
// The repo and role pair selects from a number of profiles in the config. The 'view'
// selects which particular server side ref visibility profile will be used when we
// connect. Currently the 'option' portion is not implemented, however it is documented
// here to note that it may be added in the future.
//
type TwgitURL struct {
	URL *url.URL
}

func NewTwgitURL(u *url.URL) (tw *TwgitURL, err error) {
	if u.Scheme != "twgit" {
		return nil, errors.Errorf("the url %#v does not have a 'twgit' scheme", u)
	}

	segpath := segmentPath(u)
	if len(segpath) == 0 {
		return nil, errors.Errorf(
			"the url %#v must have at least one path component specifying the role", u)
	}

	return &TwgitURL{u}, nil
}

// ParseTwgitURL parses string s and returns a TwgitURL or error
func ParseTwgitURL(s string) (tw *TwgitURL, err error) {
	u, err := url.Parse(s)
	if err != nil {
		return nil, errors.Wrapf(err, "failed to parse string %#v as a twgit url", s)
	}
	return NewTwgitURL(u)
}

func segmentPath(u *url.URL) []string {
	path := u.Path

	switch {
	case path == "":
		return nil
	case strings.HasPrefix(u.Path, "/"):
		path = path[1:]
	}
	return strings.SplitN(path, "/", -1)
}

// splits the path portion of the url on the '/' characters.
// Given twgit://foo/bar/baz this method will return []string{"bar", "baz"}
func (t *TwgitURL) segmentPath() []string {
	return segmentPath(t.URL)
}

func (t *TwgitURL) Repo() string { return t.URL.Host }
func (t *TwgitURL) Role() string { return t.segmentPath()[0] }
func (t *TwgitURL) View() string {
	xs := t.segmentPath()
	if len(xs) > 1 {
		return xs[1]
	} else {
		return ""
	}
}

// Options returns the Query string section of the URL as a url.Values object
func (t *TwgitURL) Options() url.Values { return t.URL.Query() }
