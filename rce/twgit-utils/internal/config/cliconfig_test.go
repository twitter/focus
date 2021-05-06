package config

import (
	"bytes"
	"fmt"
	"testing"

	"git.twitter.biz/focus/rce/twgit-utils/internal/common"
	"git.twitter.biz/focus/rce/twgit-utils/internal/userhash"
)

func TestCLIConfigFromEnv(t *testing.T) {
	f := NewFixture(t)
	defer f.Close()

	pairs := []string{
		"TWGIT_USER", "youzorr",
		"USER", "yz",
		"TWGIT_CONFIG", "/path/to/config.yaml",
		"TWGIT_ROLE", "roll",
		"TWGIT_REPO", "repo",
		"TWGIT_DEBUG", "true",
		"TWGIT_TRACE", "true",
		"TWGIT_QUIET", "true",
		"TWGIT_FORCE", "true",
		"TWGIT_TEST", "true",
		"TWGIT_HASHER_PARTITION_COUNT", "5432",
		"TWGIT_HASHER_REPLICATION_FACTOR", "4321",
		"TWGIT_HASHER_LOAD", "1.337",
		"IGNORED", "WHAT",
	}

	var c, d *CLIConfig
	var err error

	envVisit := common.NewPairsVisitor(pairs...)

	c, err = NewCLIConfigFromEnv(envVisit)
	f.NoError(err)

	expect := &CLIConfig{
		ConfigPath: "/path/to/config.yaml",
		Debug:      true,
		Trace:      true,
		Quiet:      true,
		User:       "youzorr",
		Force:      true,
		Hasher: userhash.HasherConfig{
			PartitionCount:    5432,
			ReplicationFactor: 4321,
			Load:              1.337,
		},
		Repo: "repo",
		Role: "roll",
		Test: true,
	}

	f.Equal(expect, c)

	// remove TWGIT_USER var and see if it picks up USER
	d, err = NewCLIConfigFromEnv(
		common.NewPairsVisitor(
			pairs[2:]...,
		),
	)

	f.NoError(err)

	expect.User = "yz"

	f.Equal(expect, d)
}

func TestErroneousValues(t *testing.T) {
	f := NewFixture(t)
	defer f.Close()

	runInt := func(k, v, typestr string) {
		c, err := NewCLIConfigFromEnv(
			common.NewPairsVisitor(k, v),
		)
		f.Nil(c)
		f.Error(err)

		f.Contains(err.Error(),
			fmt.Sprintf("the env var \"%s\" "+
				"was set to value \"%s\" which could not be converted to %s",
				k, v, typestr))
	}

	runInt("TWGIT_HASHER_PARTITION_COUNT", "potato", "an int")
	runInt("TWGIT_HASHER_REPLICATION_FACTOR", "potato", "an int")
	runInt("TWGIT_HASHER_LOAD", "potato", "a float64")

}

func TestLogConfig(t *testing.T) {
	f := NewFixture(t)
	defer f.Close()

	cc := &CLIConfig{
		Debug: true,
		Trace: true,
	}

	var buf bytes.Buffer

	lc := NewLogConfig(cc, &buf)

	f.True(lc.IsDebug())
	f.True(lc.IsTrace())
	f.Same(&buf, lc.Output())
}

func TestCLIConfigDefaults(t *testing.T) {
	f := NewFixture(t)
	defer f.Close()

	cc := &CLIConfig{
		Hasher: userhash.HasherConfig{
			PartitionCount:    9973,
			ReplicationFactor: 34,
			Load:              1.25,
		},
		Repo: "source",
		Role: "dev",
	}

	a := new(CLIConfig)
	CLIConfigDefaults(a)
	f.Equal(cc, a)
}

func TestMergeCLIConfig(t *testing.T) {
	f := NewFixture(t)
	defer f.Close()

	a := &CLIConfig{
		ConfigPath: "/path/to/config.yaml",
		Debug:      true,
		Trace:      true,
		Quiet:      true,
		User:       "youzorr",
		Force:      true,
		Hasher: userhash.HasherConfig{
			PartitionCount:    5432,
			ReplicationFactor: 4321,
			Load:              1.337,
		},
		Repo: "",
		Role: "roll",
		Test: true,
	}

	b := &CLIConfig{
		Repo: "depot",
		User: "THIS WILL NOT BE MERGED",
	}

	f.NoError(a.Merge(*b))

	f.Equal(
		&CLIConfig{
			ConfigPath: "/path/to/config.yaml",
			Debug:      true,
			Trace:      true,
			Quiet:      true,
			User:       "youzorr",
			Force:      true,
			Hasher: userhash.HasherConfig{
				PartitionCount:    5432,
				ReplicationFactor: 4321,
				Load:              1.337,
			},
			Repo: "depot",
			Role: "roll",
			Test: true,
		},
		a,
	)
}
