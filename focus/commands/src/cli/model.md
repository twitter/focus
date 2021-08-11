# Repository


$repo/
    focus/
        mandatory.layers.json
        project/
            <name>.layers.json
        
    .focus/
        user.selection.json



We must record content hashes to know whether the selection file itself has changed.
If the file changes, we must recompute the contents of the working tree.


```
{ 
    "entries": [
        { "name": "", content_hash: "blah" },
    ],
}
```


## Reapplication

Conditions under which we must reapply:

- The layers underlying the selection change
    - We can determine whether this has happened by taking checksum of the layers
- We merge changes from upstream
- Any of the build targets referenced from the layers change

- Use generalized tracked file checksums?


## Might need a daemon?

Watching for files that have been modified


fd '(BUILD\.?*|WORKSPACE\.?|.*\.bzl)'

fd -t f '(BUILD\.?*|WORKSPACE\.?|.*\.bzl)' | sort -s 
If modified since


<!-- Shape of the glob -->
