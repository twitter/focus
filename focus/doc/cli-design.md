# CLI design sketches

## Cloning a repo

focus <object> <verb>

`> focus branch search user:wjb`
```
wjb/foo
wjb/bar
wjb/baz
```

### Interaction with Git Federation?
Branch discovery from the archival repo that you don't have locally?
`> focus branch search hootenanny --archive`


### PFD: Finding profiles that apply to the main branch

We imagine that we 

`> focus profile search`
```
omakase
timelinemixer
workflows/cdpain
zeppole
```

`> focus profile search workflows`
```
workflows/cdpain
```

`> focus profile show workflows/cdpain`
```
Profile cdpain
~~~~~~~~~~~~~~

Coordinates
===========
//workflows/examples/cdpain/...

Attributes
==========
hashtag:cdpain
hashtag:workflows
```

### Managing local repositories

`> focus repo list`
```
kind full path /Users/wilhelm/workspace/fullsrc branch wjb/foo (status: dirty / detached / ...?)
kind sparse path /Users/wilhelm/workspace/foo profile cdpain branch wjb/foo (status: dirty / detached / ...?)
kind sparse path /Users/wilhelm/workspace/bar profile timelineservice branch wjb/bar
kind archive path /Users/wilhelm/workspace/archive profile branch timelinecocktail/algorithmic
```

`> focus repo create ~/workspace/foo profile cdpain`
```
Cloning into ~/workspace/foo (from branch master)

Checked out 3% of files
```

### PFD: Scope modification

#### Expanding scope

#### Stack based scope



