package cmd

import (
	"os"

	"git.twitter.biz/focus/rce/twgit-utils/internal/common"
	"git.twitter.biz/focus/rce/twgit-utils/internal/hooks/pre_push"
	"git.twitter.biz/focus/rce/twgit-utils/internal/hooks/update"
	"git.twitter.biz/focus/rce/twgit-utils/internal/resolver"
	"git.twitter.biz/focus/rce/twgit-utils/internal/unwinder"

	"github.com/pkg/errors"
	"github.com/spf13/cobra"
)

func prePushHookCmd(setup *cmdSetup) *cobra.Command {
	return &cobra.Command{
		Use:   "pre-push name location",
		Short: "see githooks(5) for details",
		Args:  cobra.ExactArgs(2),
		RunE: func(cc *cobra.Command, args []string) error {
			return unwinder.Run(func(u *unwinder.U) {
				loaded, err := setup.LoadMainConfigFile()
				u.Check(err)

				r := resolver.NewResolver(setup.CLI.User, loaded.Config)

				u.Check(pre_push.Run(&pre_push.PrePushConfig{
					Input:      cc.InOrStdin(),
					Err:        cc.ErrOrStderr(),
					RemoteName: args[0],
					RemoteDest: args[1],
					Resolver:   r,
				}))
			})
		},
	}
}

func updateHookCmd(setup *cmdSetup) *cobra.Command {
	return &cobra.Command{
		Use:   "update ref-name old-obj new-obj",
		Short: "called on the server side for each ref update",
		Long: `
from githooks(5):

	This hook is invoked by git-receive-pack(1) when it reacts to git push
	and updates reference(s) in its repository. Just before updating the
	ref on the remote repository, the update hook is invoked. Its exit
	status determines the success or failure of the ref update.

	A zero exit from the update hook allows the ref to be updated. Exiting
	with a non-zero status prevents git receive-pack from updating that
	ref.

This update hook can enforce several rules, and those rules can be enabled
by setting environment variables, or values in the git config of the
repository where the hook resides. (note that the config keys are
case-insensitive, as far as the hook is concerned, they'll all be normalized
to all lower case.) Valid true boolean values are: 1, t, T, TRUE, true, True,
and valid false boolean values are: 0, f, F, FALSE, false, False.

* If the config value 'twgit.updateHook.checkOwnerMatches' or the env var
  TWGIT_CHECK_OWNER_MATCHES is set to a true value, then the hook will ensure
	that the REMOTE_USER env var is set and that the first path component of
	the ref being pushed matches that REMOTE_USER value. This is to enforce the
	rule that in some repos, users are only allowed to modify their own refs,
	and those refs must be located in a directory named with their username.

	For example:

		if REMOTE_USER == "alice", and the ref is "refs/heads/alice/foobar", the
		update will be allowed.

		if REMOTE_USER == "alice" and the ref is "refs/heads/bob/foobar", then the
		update will not be allowed.

	If the update is not allowed, only that ref fails to update, and if there are
	other refs in the push operation, they will not be affected by this result.

* If the config value 'twgit.updateHook.tagCreateOrUpdateForbidden' or the env var
	TWGIT_TAG_CREATE_OR_UPDATE_FORBIDDEN is set to a true value, then the hook will
	ensure that tags (i.e. refs starting with "refs/tags/") cannot be either created
	or updated in this repository. Note that they still may be deleted.

In the case that the update hook rejects a change, a diagnostic message will be printed
to stderr, which git will then show to the user.
`,
		Args: cobra.ExactArgs(3),
		RunE: func(cc *cobra.Command, args []string) error {
			var ruser string
			var ok bool
			if ruser, ok = setup.LookupEnv("REMOTE_USER"); !ok {
				return errors.Errorf("expected REMOTE_USER to be set but it was not")
			}

			uha := &update.UpdateHookArgs{
				RefName:    args[0],
				OldOid:     args[1],
				NewOid:     args[2],
				RemoteUser: ruser,
			}

			update.LoadUpdateHookArgsFromEnv(common.NewEnvVisitor(os.Environ()), uha)

			return update.Run(uha)
		},
	}
}

func HookCmd(setup *cmdSetup) *cobra.Command {
	hookCmd := &cobra.Command{
		Use:    "hook",
		Short:  "implementation of githooks(5)",
		PreRun: setup.LogSetupHook(),
	}

	hookCmd.AddCommand(
		prePushHookCmd(setup),
		updateHookCmd(setup),
	)

	return hookCmd
}
