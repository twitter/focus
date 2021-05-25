#include <stdarg.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdlib.h>

int git_storage_init(void *attachment,
                     const char *repo_path,
                     size_t repo_path_length,
                     const char *fifo_path,
                     size_t fifo_path_length,
                     const char *args,
                     size_t args_length,
                     size_t hash_raw_bytes);

int git_storage_shutdown(void *attachment);

int git_storage_fetch_object(void *attachment,
                             const unsigned char *oid,
                             const char *path,
                             size_t path_length,
                             off_t offset,
                             size_t capacity,
                             off_t *header_offset,
                             size_t *header_length,
                             off_t *content_offset,
                             size_t *content_length,
                             size_t *total_length,
                             size_t *new_capacity,
                             time_t *atime,
                             time_t *mtime);

int git_storage_size_object(void *attachment,
                            const unsigned char *oid,
                            size_t *size,
                            time_t *atime,
                            time_t *mtime);

int git_storage_write_object(void *attachment,
                             const unsigned char *oid,
                             const unsigned char *header,
                             size_t header_length,
                             const unsigned char *body,
                             size_t body_length,
                             time_t mtime);
