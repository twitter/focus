# Project Cache 

The project cache is a caches pattern sets for projects defined in a repository. It can be used in repositories where the selection includes only predefined projects to skip slower synchronization mechansisms. 

It is keyed on a _build graph hash_, a digest of all of the build-relevant files in the tree. The cache also contains records mapping commit IDs to build graph hashes to reduce the need to perform that computation unless the build graph has been locally modified. Calculating this hash with Twitter's ~24GiB object database takes about approximately 9 seconds on a MacBook Pro (16-inch, 2019) with the 2.4 GHz 8-Core Intel Core i9 processor. N.B.: we aren't taking full advantage of these precomputed commit to build graph hashes since we don't have a mechanism for fetching them yet. 

## Using Project Cache
On the workstation, the selection can only contain projects. You should set up a CI job with multiple shards as described in [generating and pushing](#generating-and-pushing) and configure clients with the appropriate endpoint -- see [configuration](#configuration).

## Conceptual Schema
```
  [Commit ID] -> [Build Graph Hash (SHA-256)] -+-> [Project A]
                                               |       \
                                               |        `-> [Pattern Set]
                                               |
                                               +-> [Project B]
                                               |     | 
                                               .     `-> [Pattern Set]
                                               .
                                               . 
```

## Configuration
The cache endpoint is specified in the  Git configuration as the `focus.project-cache-endpoint` variable. If configured, Focus will attempt to fetch from this endpoint cache content for the commit it is trying to sync if it is not already present in the local project cache database.

## Content Storage
Content is stored on an HTTP server using a simple scheme. It uses `PUT` and `GET` to store and fetch data. Only repos generating content need to be able to perform `PUT` requests against the endpoint. 

## Generating and pushing 
The `focus project-cache` command allows you to interact with the cache. The `focus project-cache push` command will generate cache content and push it to the given endpoint. Index generation is sharded and the different shards can be calculated by separate machines every time a commit lands at the head of your repository. 

You could set up a CI job with two different shards like so:

> On CI machine #1:
> `focus project-cache push --shard-count=2 --shard-index=0`
> 
> On CI machine #2:
> `focus project-cache push --shard-count=2 --shard-index=1`


Any number of shards can be used.

The endpoint path must refer to an existing directory on the server. Files are written there forming a flat namespace. Each repository should have a different endpoint path.

Care should be taken to expire content so that your server's disk does not fill up. Delete files whose creation time preceeds the time window your sparse repos HEAD commits map to: running a simple command such as `find $endpoint_path -ctime +7 -delete` should work well for cleaning up the path on a plain HTTP server storing and serving files from disk; in this case deleting all files older than one week. You might want to delete the manifest files ending in the pattern `.manifest_v*.json` first to prevent errors from Focus repos that are fetching.
