package cmd

import "testing"

func TestCmdSetupLookupEnv(t *testing.T) {
	f := NewFixtureNoRepo(t)
	defer f.Close()

	cs := &cmdSetup{
		env: []string{
			"SHELL=/bin/zsh",
		},
	}

	v, ok := cs.LookupEnv("SHELL")
	f.True(ok)
	f.Equal("/bin/zsh", v)

	v, ok = cs.LookupEnv("LC_ALL")
	f.False(ok)
	f.Empty(v)
}
