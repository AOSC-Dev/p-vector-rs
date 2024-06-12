# Database Schema

## pv_repos

Track debian repos.

```sql
create table pv_repos
(
    -- repo name e.g. amd64/ruby-3.3.0
    -- repo = arch/branch or component-arch/branch
    name         text                     not null
        primary key,
    -- path under dists or pool e.g. ruby-3.3.0/main
    -- path = branch/component
    path         text                     not null,
    -- 0 for stable, 1 for non-stable topic
    testing      integer                  not null,
    -- git branch e.g. ruby-3.3.0
    branch       text                     not null,
    -- component e.g. main
    component    text                     not null,
    -- arch e.g. amd64
    architecture text                     not null,
    -- modified time
    mtime        timestamp with time zone not null
);
```

## pv_packages

Track debian packages: uniquely identified by `(package name, version, repo)`.

```sql
create table pv_packages
(
    -- package name e.g. aarty
    package        text                         not null,
    -- package version e.g. 0.6.1
    version        text                         not null,
    -- repo name, match pv_repos
    repo           text                         not null
        constraint fkey_repo
            references pv_repos
            on delete cascade
            deferrable initially deferred,
    -- arch e.g. amd64
    architecture   text                         not null,
    -- path under debs e.g. pool/branch/main/p/pkg_ver_amd64.deb
    filename       text                         not null,
    -- size in bytes
    size           bigint                       not null,
    -- sha256 hash
    sha256         text                         not null,
    -- modified time in unix epoch
    mtime          integer                      not null,
    -- deb control.tar last modified time
    debtime        integer                      not null,
    -- deb package section
    section        text default 'unknown'::text not null,
    -- deb installed size
    installed_size bigint                       not null,
    -- deb maintainer
    maintainer     text                         not null,
    -- deb description
    description    text                         not null,
    -- compress version for sorting, see comparable_dpkgver function
    _vercomp       text                         not null,
    primary key (package, version, repo)
);
```

## pv_package_dependencies

Track package dependencies.

```sql
create table pv_package_dependencies
(
    -- package name, match pv_packages
    package      text not null,
    -- package version, match pv_packages
    version      text not null,
    -- package repo, match pv_packages
    repo         text not null,
    -- deb package relationship e.g. Depends, Breaks, Conflicts
    relationship text not null,
    -- deb package dependency e.g. gcc-runtime (>= 13.2.0-2), glibc (>= 1:2.37-1)
    value        text not null,
    primary key (package, version, repo, relationship),
    constraint fkey_package
        foreign key (package, version, repo) references pv_packages
            on delete cascade
            deferrable initially deferred
);
```

## pv_package_files

Trace package contents.

```sql
create table pv_package_files
(
    -- package name, match pv_packages
    package text,
    -- package version, match pv_packages
    version text,
    -- package repo, match pv_packages
    repo    text,
    -- relative parent folder in deb content
    path    text,
    -- file name in deb content
    name    text,
    -- size in bytes
    size    bigint,
    -- file type, see unix file type enums
    ftype   smallint,
    -- file permission in octal
    perm    integer,
    -- unix user id
    uid     bigint,
    -- unix group id
    gid     bigint,
    -- user owner name
    uname   text,
    -- group owner name
    gname   text,
    constraint fkey_package
        foreign key (package, version, repo) references pv_packages
            on delete cascade
            deferrable initially deferred
);
```

## pv_package_sodep

Track package shared library provides and dependencies.

```sql
create table pv_package_sodep
(
    -- package name, match pv_packages
    package text,
    -- package version, match pv_packages
    version text,
    -- package repo, match pv_packages
    repo    text,
    -- 0 if the package provides, 1 if the package requires
    depends integer,
    -- .so name excluding version suffix e.g. libc.so
    name    text,
    -- .so version e.g. .6
    ver     text,
    constraint fkey_package
        foreign key (package, version, repo) references pv_packages
            on delete cascade
            deferrable initially deferred
);
```

## Foreign tables from abbs-meta

- trees
- tree_branches
- packages
- package_duplicate
- package_versions
- package_spec
- package_dependencies
- dpkg_repo_stats

Use pv_dbsync to record last sync of abbs.db.

## Foreign tables from piss

- upstream_status
- package_upstream
- anitya_link
- anitya_projects

## v_packages_new

Find the packages with latest version (i.e. skip old versions). Implemented by ordering by `_vercomp` and then `SELECT DISTINCT ON` to use the latest one.

Does not consider case when a package in `all` is newer than the one in `amd64`.

```sql
create materialized view v_packages_new as
SELECT DISTINCT ON (pv_packages.repo, pv_packages.package) pv_packages.package,
                                                           pv_packages.version,
                                                           pv_packages.repo,
                                                           pv_packages.architecture,
                                                           pv_packages.filename,
                                                           pv_packages.size,
                                                           pv_packages.sha256,
                                                           pv_packages.mtime,
                                                           pv_packages.debtime,
                                                           pv_packages.section,
                                                           pv_packages.installed_size,
                                                           pv_packages.maintainer,
                                                           pv_packages.description,
                                                           pv_packages._vercomp
FROM pv_packages
WHERE pv_packages.debtime IS NOT NULL
ORDER BY pv_packages.repo, pv_packages.package, pv_packages._vercomp DESC;
```

## v_dpkg_dependencies

Queries dpkg dependencies.

```sql
create materialized view v_dpkg_dependencies as
SELECT q3.package,
       q3.version,
       q3.repo,
       q3.relationship,
       -- dependency index
       -- e.g. gcc-runtime is the first, nr=1; glibc is the second, nr=2
       q3.nr,
       -- extract fields from e.g. gcc-runtime (>= 13.2.0-2)
       -- e.g. gcc-runtime
       q3.depspl[1]                     AS deppkg,
       -- e.g. null since unset
       q3.depspl[2]                     AS deparch,
       -- e.g. >=
       q3.depspl[3]                     AS relop,
       -- e.g. 13.2.0-2
       q3.depspl[4]                     AS depver,
       -- converted depver
       comparable_dpkgver(q3.depspl[4]) AS depvercomp
FROM (SELECT q2.package,
             q2.version,
             q2.repo,
             q2.relationship,
             q2.nr,
             regexp_match(q2.dep, ('^\s*([a-zA-Z0-9.+-]{2,})(?::([a-zA-Z0-9][a-zA-Z0-9-]*))?'::text ||
                                   '(?:\s*\(\s*([>=<]+)\s*([0-9a-zA-Z:+~.-]+)\s*\))?(?:\s*\[[\s!\w-]+\])?'::text) ||
                                  '\s*(?:<.+>)?\s*$'::text) AS depspl
    -- handle OR dependencies
      FROM (SELECT q1.package,
                   q1.version,
                   q1.repo,
                   q1.relationship,
                   q1.nr,
                   unnest(string_to_array(q1.dep, '|'::text)) AS dep
            -- collect dpkg dependencies
            FROM (SELECT d.package,
                         d.version,
                         d.repo,
                         d.relationship,
                         v.nr,
                         v.val AS dep
                  FROM pv_package_dependencies d
                           JOIN v_packages_new n USING (package, version, repo)
                           -- expand e.g. gcc-runtime (>= 13.2.0-2), glibc (>= 1:2.37-1)
                           -- into separate rows
                           JOIN LATERAL unnest(string_to_array(d.value, ','::text)) WITH ORDINALITY v(val, nr)
                                ON true) q1) q2) q3;
```

