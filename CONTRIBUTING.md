# How to contribute

`focus` development is done entirely on Github. To discuss, use the [issue tracker](https://github.com/twitter/focus/issues) or [discussion board](https://github.com/twitter/focus/discussions).

To contribute, open a new pull request against the `main` branch. As part of the pull request, you will be prompted by a bot to sign our CLA if you haven't done so already.

To build and test the code, run

```sh
$ cargo test --workspace
```

If you create a new file, you should add a license header. (This will be checked in CI.) You can do this with the following script:

```sh
$ scripts/add-license.sh
```
