# Focus State Sync / Profile Application


## Ideas

- Complain in Bazel / calculate the stuff there

- Upstream changes that complain when the underyling profile changes and has not been reapplied

- Merge hook
	- How does IntelliJ do merges?
- Clippy + repo daemon watching files
	- Clippy reminds tells you to sync your project using desktop notifications or somesuch.

### Underpinnings
- `git diff --name-only <last_application_point>..`

