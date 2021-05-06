package config

import (
	"io"
	"strconv"
	"strings"

	"github.com/pkg/errors"

	"git.twitter.biz/focus/rce/twgit-utils/internal/common"
	"git.twitter.biz/focus/rce/twgit-utils/internal/userhash"
	"github.com/imdario/mergo"
)

type (
	CLIConfig struct {
		ConfigPath string
		Debug      bool
		Trace      bool
		Quiet      bool
		User       string
		Force      bool
		Hasher     userhash.HasherConfig
		Repo       string
		Role       string
		// Test should be true when we're running tests against the CLI
		Test bool
	}

	logConfig struct {
		cli *CLIConfig
		out io.Writer
	}
)

func NewLogConfig(cli *CLIConfig, logout io.Writer) common.LogConfig {
	return &logConfig{cli, logout}
}

func (lc *logConfig) IsDebug() bool     { return lc.cli.Debug }
func (lc *logConfig) IsTrace() bool     { return lc.cli.Trace }
func (lc *logConfig) Output() io.Writer { return lc.out }

func intEnv(k, v string) (i int, err error) {
	if i, err = strconv.Atoi(v); err != nil {
		return 0, errors.Wrapf(err,
			"the env var %#v was set to value %#v which could not be converted to an int", k, v)
	}
	return i, nil
}

func floatEnv(k, v string) (f float64, err error) {
	if f, err = strconv.ParseFloat(v, 64); err != nil {
		return 0.0, errors.Wrapf(err,
			"the env var %#v was set to value %#v which could not be converted to a float64", k, v)
	}
	return f, nil
}

func CLIConfigDefaults(c *CLIConfig) {
	c.Hasher.PartitionCount = 9973
	c.Hasher.ReplicationFactor = 34
	c.Hasher.Load = 1.25
	c.Repo = "source"
	c.Role = "dev"
}

func LoadCLIConfigFromEnv(envVisitor common.KeyValueVisitor, c *CLIConfig) (err error) {
	var twgUser, user string

	err = envVisitor(func(k, v string) (ierr error) {
		// slight optimization over doing a string comparison
		// with all of the items in the following switch statement
		if !(k == "USER" || strings.HasPrefix(k, "TWGIT_")) {
			return nil
		}

		switch k {
		case "TWGIT_USER":
			twgUser = v
		case "USER":
			user = v
		case "TWGIT_CONFIG":
			c.ConfigPath = v
		case "TWGIT_ROLE":
			c.Role = v
		case "TWGIT_REPO":
			c.Repo = v
		case "TWGIT_DEBUG":
			c.Debug = true
		case "TWGIT_TRACE":
			c.Trace = true
		case "TWGIT_QUIET":
			c.Quiet = true
		case "TWGIT_FORCE":
			c.Force = true
		case "TWGIT_TEST":
			c.Test = true
		case "TWGIT_HASHER_PARTITION_COUNT":
			c.Hasher.PartitionCount, ierr = intEnv(k, v)
		case "TWGIT_HASHER_REPLICATION_FACTOR":
			c.Hasher.ReplicationFactor, ierr = intEnv(k, v)
		case "TWGIT_HASHER_LOAD":
			c.Hasher.Load, ierr = floatEnv(k, v)
		default:
		}

		return ierr
	})

	switch {
	case twgUser != "":
		c.User = twgUser
	case user != "":
		c.User = user
	}

	return err
}

func NewCLIConfigFromEnv(envVisitor common.KeyValueVisitor) (c *CLIConfig, err error) {
	c = new(CLIConfig)
	if err = LoadCLIConfigFromEnv(envVisitor, c); err != nil {
		return nil, err
	}
	return c, nil
}

// Merge the values from o onto the receiver. this mutates the receiver
// when a field is using the default values
func (c *CLIConfig) Merge(o CLIConfig) (err error) {
	// if the merge succeeds errors.Wrapf returns nil
	return errors.Wrapf(
		mergo.Merge(c, o),
		"failed to merge %#v and %#v", c, o,
	)
}
