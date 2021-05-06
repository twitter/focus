package cmd

import (
	"fmt"
	"os"
	"strings"

	"git.twitter.biz/focus/rce/twgit-utils/internal/common"
	"git.twitter.biz/focus/rce/twgit-utils/internal/config"
	"git.twitter.biz/focus/rce/twgit-utils/internal/domain"
	"git.twitter.biz/focus/rce/twgit-utils/internal/git"
	"github.com/pkg/errors"
	log "github.com/sirupsen/logrus"
	"github.com/spf13/cobra"
)

type (
	cmdSetup struct {
		Repo git.LazyGit
		CLI  config.CLIConfig
		env []string
	}

	CommandHookFn func(*cobra.Command, []string)
)

const (
	ConfigPathIsBlobValue = "the-config-is-a-blob-in-the-git-repo"
	ConfigPathKey         = "twgit.cmd.config"
	DebugKey              = "twgit.cmd.debug"
	DefaultFlagPrefix     = "twgit.cmd"
	ForceKey              = "twgit.cmd.force" // forsky: russian for "force"
	HasherConfigPrefix    = "twgit.hasher"
	QuietKey              = "twgit.cmd.quiet"
	RepoKey               = "twgit.cmd.repo"
	RoleKey               = "twgit.cmd.role"
	TraceKey              = "twgit.cmd.trace"
)

func LogSetup(lc common.LogConfig) {
	log.SetFormatter(&log.TextFormatter{
		PadLevelText:           true,
		DisableLevelTruncation: true,
		TimestampFormat:        "2006-01-02T15:04:05.000000",
		FullTimestamp:          true,
	})
	log.SetOutput(lc.Output())
	if lc.IsDebug() {
		log.SetLevel(log.DebugLevel)
	}
	if lc.IsTrace() {
		log.SetLevel(log.TraceLevel)
		os.Setenv("GIT_TRACE2", "1")
	}
	// this is severely cheezy but it helps to make sure the flags are working
	if _, ok := os.LookupEnv("TWGIT_TEST"); ok {
		log.Debug("this is a debug message")
		log.Trace("this is a trace message")
	}
}

func (c *cmdSetup) Env() (osenv []string) {
	osenv = make([]string, len(c.env))
	copy(osenv, c.env)
	return osenv
}

func (c *cmdSetup) EnvVisitor() common.KeyValueVisitor {
	return common.NewEnvVisitor(c.env)
}

// this is O(n) but whatever
func (c *cmdSetup) LookupEnv(key string) (v string, ok bool) {
	for _, kv := range c.env {
		if i := strings.Index(kv, "="); i >= 0 {
			if k := kv[0:i]; k == key {
				return kv[i+1:], true
			}
		}
	}
	return "", false
}


func (c *cmdSetup) LogSetupHook() CommandHookFn {
	return func(cc *cobra.Command, args []string) {
		LogSetup(config.NewLogConfig(&c.CLI, cc.ErrOrStderr()))
	}
}

func (c *cmdSetup) LoadBlobConfig() *config.BlobConfig {
	rc := config.DefaultRefConfig

	common.CheckErr(config.LoadRefConfigFromGit(c.Repo().Config().Visit, &rc))
	common.CheckErr(config.LoadRefConfigFromEnv(c.EnvVisitor(), &rc))

	return (&rc).BlobConfig(c.Repo())
}

type loadedConfig struct {
	text   string
	Config *domain.Config
}

func (lc *loadedConfig) String() string { return lc.text }

// LoadMainConfigFile determines the location of the config file (twgit.yaml)
// and unmarshals a domain.Config struct using it. It returns a *loadedConfig
// that will have a copy of the text of the config and also the loaded/validated
// config itself.
func (c *cmdSetup) LoadMainConfigFile() (loaded *loadedConfig, err error) {
	var configPath string

	loaded = &loadedConfig{}

	if configPath = c.CLI.ConfigPath; configPath == "" {
		// reads the twgit config out of the repo
		bc := c.LoadBlobConfig()

		if loaded.text, err = bc.ReadString(); err != nil {
			return nil, errors.Wrap(err, "failed to read config out of git")
		}
	} else {
		var data []byte

		if data, err = os.ReadFile(configPath); err != nil {
			return nil, errors.Wrapf(err, "failed to read file at path %#v", configPath)
		}
		loaded.text = string(data)
	}

	if loaded.Config, err = domain.LoadConfigFromYaml([]byte(loaded.text)); err != nil {
		return nil, err
	}

	return loaded, nil
}

func (c *cmdSetup) Sayln(cc *cobra.Command, a ...interface{}) (n int, err error) {
	if !c.CLI.Quiet {
		return fmt.Fprintln(cc.ErrOrStderr(), a...)
	}
	return 0, nil
}

// ConfigFlag registers the --config flag with the correct defaults. We use this
// in multiple different subcommands, so it's defined in one place
func (c *cmdSetup) ConfigFlag(cc *cobra.Command) {
	cc.PersistentFlags().StringVarP(&c.CLI.ConfigPath,
		"config", "f", c.CLI.ConfigPath,
		"path to the config file to use (default is to read out of the git repo)")
}

func (c *cmdSetup) UserFlag(cc *cobra.Command) {
	cc.PersistentFlags().StringVarP(&c.CLI.User,
		"user", "u", c.CLI.User, "enable visibility for username's refs on backend")
}

func (c *cmdSetup) RepoFlag(cc *cobra.Command) {
	cc.Flags().StringVar(&c.CLI.Repo,
		"repo", c.CLI.Repo, "the name of the profile you want to use the nodes from")
}

func (c *cmdSetup) RoleFlag(cc *cobra.Command) {
	cc.Flags().StringVar(&c.CLI.Role,
		"role", c.CLI.Role, "the name of the role you want to use the nodes from in the repo's profile")
}
