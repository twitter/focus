### Focus Improvement Plan - Q3 & Q4 2022 ###

# Twitter-specific improvements

## Implement integration tests in Source

Increase the stability of sparse Source clones by adding a test suite that will fail if predefined projects are not usable. Right now, if new implicit dependencies are introduced in the Source repo, sparse checkouts might stop working without our noticing before users complain. We should have a battery of tests that makes sure that baseline the checkout and projects are usable.

We may be able to get this as a side-effect of implementing Speed `focus sync` through deployment of the [project cache](#project-cache) since it requires outlining all defined projects.

This make Focus more stable to use within the company, and hence easier to adopt.

# General improvements

## Project Cache

### Speed `focus sync` through depoloyment of project cache

The project cache allows us to compute sparse pattern sets for projects defined in `$repo/focus/projects/` as a CI step for each commit that hits the main branch. Computation of this cache content can be sharded. Clients download these predefined pattern sets over HTTP. In sparse repos with no ad-hoc Bazel or directory targets selected, these precomputed pattern sets can be used to skip normal synchronziation by applying the union of all selected projects as the pattern set. This makes sync quite quick and it is plausible to perform in-line of Git merge or rebase operations, which decreases one of the only remaining Focus-related eccentriticies in the Git workflow.

This makes Focus faster to use and easier to adopt.

## Background sync

### Speed synchronization by deploying background sync

Test and enable background syncing, a feature that is already developed (accessible through `focus background {enable,disable}`), which performs syncing in the background after new content has been fetched from the repo. 

We need to test that the idle detection is working, and probably build out an analogous command for Linux.

We could improve it further by suspending work if we detect that the computer is busy while performing outlining or other heavyweight operations.

This makes Focus easier to adopt.


## UX

### Eliminate Git workflow eccentricities

Using `focus` should feel natural. Rather than having to manually `focus sync` after merge or rebase operations, it should be run from a hook. We already have a skeletal `focus event` command. We should flush it out into a complete implementation, test it, and hook it up.

This makes Focus easier to adopt.


### Consider a more in-depth UI

We have started running up against limits of what we can achieve as a CLI. Consider the case of the `.focus/preflight` script, which is meant to be included in Bazel wrappers for focused repositories:  we do desktop notifications as well as console logging from the `focus detect-build-graph-changes`, but the notifications on macOS appear to come from Finder rather than focus itself. What would be better in this case is IDE integration for relevant IDEs like VS Code and IntelliJ IDEA.

In addition to providing better visibility into Focus state during builds, we could also integrate a project finder analogous to the exisitng CLI interactive chooser present in `focus add -i`.

This makes `focus` easier to use.


## Platform support

### Implement background job scheduling for Linux

The Linux `focus` suite does not support background tasks right now. We should port our scheduling code to `systemd` or figure out how to use normal `git maintenance` to schedule the work (the macOS version suffers bugs that made it unsuitable, but Linux likely works).
