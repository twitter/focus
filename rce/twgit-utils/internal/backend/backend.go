package backend

import (
	"fmt"
	"os"
	"os/exec"
	re "regexp"
	"strings"
)

type (
	BackendCmd struct {
		Config
	}
)

func New(config *Config) (bcf *BackendCmd, err error) {
	bcf = &BackendCmd{
		Config{
			Env:    make([]string, len(config.Env)),
			Stdin:  config.Stdin,
			Stdout: config.Stdout,
			Stderr: config.Stderr,
		},
	}

	copy(bcf.Env, config.Env)
	return bcf, nil
}

func (b *BackendCmd) CopyEnv() []string {
	c := make([]string, len(b.Env))
	copy(c, b.Env)
	return c
}

var (
	hasUserRe = re.MustCompile(
		`^/([^/]+)` + // match the first 'repository' part, like 'source.git'
			`/([^/]+)` + // a potential username or keyword like _all or _tags
			`/((?:git-(?:upload|receive)-pack)|info/refs)$`, // the git comand to run
	)
)

const xfrHideRefs = "transfer.hideRefs"

func hideRef(ref string) string {
	return fmt.Sprintf("%s=%s", xfrHideRefs, ref)
}

func (b *BackendCmd) Cmd() (cmd *exec.Cmd, err error) {
	cgi, err := NewCGIEnv(b.Env)
	if err != nil {
		return nil, err
	}

	cmd = &exec.Cmd{
		Path: cgi.GitBin,
		Args: []string{
			"git",
			"-c", hideRef("refs/heads"),
			"-c", hideRef("refs/tags"),
			"-c", hideRef("!refs/heads/master"),
			"-c", hideRef("!refs/admin"),
		},
		Env:    b.CopyEnv(),
		Stdin:  os.Stdin,
		Stdout: os.Stdout,
		Stderr: os.Stderr,
	}

	addHideRef := func(r string) {
		cmd.Args = append(cmd.Args, "-c", hideRef(r))
	}

	for _, r := range cgi.AdditionalHideRefs {
		addHideRef(r)
	}

	// we can just append the new values to this slice because
	// when there are duplicates, later entries override earlier ones
	addEnv := func(k, v string) {
		cmd.Env = append(cmd.Env, fmt.Sprintf("%s=%s", k, v))
	}

	var matches []string

	if matches = hasUserRe.FindStringSubmatch(cgi.PathInfo); matches != nil {
		user := matches[2]

		switch user {
		case "_all":
			addHideRef("!refs") // show everything
		case "_tags":
			addHideRef("!refs/tags") // show them the tags (?)
		default:
			addHideRef(
				fmt.Sprintf("!refs/heads/%s", strings.TrimPrefix(user, "~")),
			)
		}

		userSlash := fmt.Sprintf("%s/", user)

		addEnv("LC_ALL", "C")
		addEnv("PATH_INFO", strings.Replace(cgi.PathInfo, userSlash, "", 1))
		addEnv("PATH_TRANSLATED", strings.Replace(cgi.PathTranslated, userSlash, "", 1))
		addEnv("REQUEST_URI", strings.Replace(cgi.RequestURI, userSlash, "", 1))
	}

	cmd.Args = append(cmd.Args, "http-backend")
	return cmd, nil
}
