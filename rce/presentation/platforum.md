# Death to the Git Journal: Towards a Federated Architecture

## Overview

### A long long time ago...

* In 2013 there were two repos, then Wilhelm and I made them one.
* We developed and deployed a binary log for changes to the repository
* This solved the problem of git pulls being too inefficient to service the thousands of requests that were hitting the backend servers

### Some Terminology

Git is a bag of objects with named pointers **ref**erencing some of them.

* Object: a thing stored in git's content addressable store
 * Commits, Trees, Blobs, Tags
* Ref: A SHA1 that's been given a name, pointing to a Commit or a Tag

Git is very much like a garbage collected virtual machine, only slower and implemented in a combination of C89 and POSIX-sh.

* Refs are the pointers to objects in the "heap" (ODB).
* Refs are also known as "heads", as they are the head object in a directed acyclic graph of versions.
* *This is different from `HEAD`*. We will not be discussing `HEAD`.

### The 7 Year Itch

* Today we have a different problem: too much everything.

|         | 2016-12   |  2020-04   |  increase  |
|---------|-----------|------------|------------|
| num obj | 4,260,134 | 27,230,019 |    6.39x   |
| size    | 1,631,236 | 17,736,411 |   10.87x   |
| refs    |    20,954 |     90,133 |    4.30x   |
| tags    |    17,813 |     38.098 |    2.13x   |

### Why Is Git So Slow?

* This is my favorite question because I could go on for at least an hour talking about all the bad decisions in git's implementation.
* The short answer is, even if an operation is O(log n) if N gets big enough, it's gonna be slow.

### Inefficiencies Abound: The System

* Git's packfile format requires a binary search through (possibly multiple) index files to find any object.
* It then needs to perform a read at the given location and decompress the object, possibly applying a delta to the data.
* This random IO is somewhat dealable when all of the data fits in the OS page cache, less so when it's nearly half of available memory.

### A Little Bit Of Technical Detail: Git Push

Without the journal git performs what's known as "ref negotiation"

* client: "Hey I'd like to push something"
* server: "Great here's a list of all of my refs"
* client: *processing list of refs against own list of refs*
* client: "Ok, here's a file that has the absolute smallest number of objects you need in order to be fully connected when you update your ref to where I say you should"
* server: *accepting upload, checking connectivity, updating refs*
* server: cool.

In a small repo this is fast. Today, however, when you go to push to source, the server sends you a list of 147960 refs which is a 12.2 MiB payload. Every single time.

### Nothing Fails Like Success

By solving the problem of "How do we replicate this repo everywhere super fast?" we inadvertently created a new problem. We replicate everything to everyone. Additionally, due to quirks in git's implementation (described before re: minimal packs) we never actually delete any objects from any repository. There's a super nerdy technical explanation for why this is necessary, but basically it's the problem of eventual consistency.

So everyone gets everything and nothing is ever deleted. Anywhere. Ever.

### Administrivia

The other technical problem we didn't solve is that git is essentially a free-for-all. We have no rules that would help keep the system healthy and performant. Refs live forever. Tags are pushed everywhere and anywhere. People are free to create branches in whatever hierarchies they please, so long as they have a '/' in the name.

# A New Hope

There's a fairly easy answer to a large subset of these problems, and the good news is that we've never done anything to fix them, so there's a lot of room for improvement.

We can also kill the journal, allowing us to more easily keep up with OSS git and let users perform shallow clones of source.

## Get Your Shit Together

The first thing to do is establish some basic policy:

* Refs may only be created under the current user's name.
  * `jsimms/whatever`
  * `kevino/whatever-kevin-actually-does`

You'll see why this is important in a minute.

### E Pluribus Unum: From many, one.

There is no reason why we need to have only one repository that is providing "source".

Crazy nastyass git DGAF what objects or refs you put where. The concept of "a" repository is simply convention. You can push any ref from any repository *to* any repository.

Please do not try this in source or I will hunt you down.

What we need is to reduce the total number of refs any user sees when trying to perform any given operation, plus we need to reduce the growth factor of the repositories all of us use day to day.

### Role for Damage

We will compose together a number of different git repos to form "source", plus some lightweight client side and server side magic to make everything easy for users.

#### Master Only Origin

* The current "origin" will serve *one* ref: "master".
* As it is today, only CI will be allowed to write to this ref.
* Tags are forbidden.
* *This lets us spread requests across a number of read only replicas*

#### "Dev" Cluster

* There will be a cluster of repositories (currently planned ~15) for work-in-progress branches.
* Usernames will be consistently hashed and assigned a backend node.
* They will be able to write to only their server under only their name
* They'll be able to pull anyone else's branch and then push it to their space if they make changes.
* This is like github's pull request model only without forking
* There will be a server-side wrapper around the `git-http-backend` that will control visibility. Users will see only their own refs by default. This will speed up all remote operations.
* *This will reduce the total number of objects that any one repository will accumulate, and reduce the total number of refs any user must see*

#### "Dev" Cluster: TTL

* There will be a 30 day limit on dev branches.
* After 30 days the branch will be moved to the "archive" repo.
* Again, this is to keep the total number of refs and objects to a controllable level.

#### "Archive" Cluster

* There will be a cluter of repos that will hold onto refs for a longer yet possibly not indenfinite time.
* Users may not update these refs, they can only delete their own, or read any.
* Tags may live here or may be hosted on their own repo (TBD)

## Holy Shitballs That Sounds Really Complicated!

Don't worry, I got you fam.

## Introducing: twgit urls

> "That sounds like a shitty version of wily" - Wilhelm

Users will no longer use the literal backend URL to access the repos. Instead they'll use an abstract descriptor that will be resolved to a concrete URL using a config file and some scripting.

```
  twgit://repo/role/view?option=foo
```

### An example gitconfig

```
[remote "origin"]
  url = twgit://source/main

[remote "dev"]
  url = twgit://source/dev/$user

[remote "archive"]
  url = twgit://source/archive/$user
```

### Use cases

These URLs will be configured as regular git remotes for users, so most of the time users won't have to know the details.

```
# fetch changes from main
$ git fetch origin master

# Push a work in progress branch
$ git push -u dev

# Pull someone else's branch to your repo (this one is a little annoying)
$ git pull twgit://source/dev/risaacs risaacs/xyz:risaacs/xyz
```


