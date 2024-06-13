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

## v_so_breaks

Collect info where package A might break package B if package A changes its shared library version.

In another words, package A provides a shared library which is used by package B.

```sql
create materialized view v_so_breaks as
SELECT sp.package,
       sp.repo,
       sp.name    AS soname,
       sp.ver     AS sover,
       sd.ver     AS sodepver,
       sd.package AS dep_package,
       sd.repo    AS dep_repo,
       sd.version AS dep_version
-- sp (package A) provides a shared library (see WHERE clause below)
FROM pv_package_sodep sp
         JOIN v_packages_new vp USING (package, version, repo)
         -- filter sd
         JOIN pv_repos rp ON rp.name = sp.repo
         -- same arch or package B is noarch
         -- same component or package B is in main
         -- if A in topic, B in topic or stable
         -- if A in stable, B in stable
         JOIN pv_repos rd
              ON (rd.architecture = rp.architecture OR rd.architecture = 'all'::text) AND rp.testing <= rd.testing AND
                 (rp.component = rd.component OR rp.component = 'main'::text)
         -- sd (package B) depends on the shared lirbary
         JOIN pv_package_sodep sd
              ON sd.depends = 1 AND sd.repo = rd.name AND sd.name = sp.name AND sd.package <> sp.package AND
                 (sp.ver = sd.ver OR sp.ver ~~ (sd.ver || '.%'::text))
         JOIN v_packages_new vp2 ON vp2.package = sd.package AND vp2.version = sd.version AND vp2.repo = sd.repo
WHERE sp.depends = 0
UNION ALL
-- TODO: analyze pv_package_issues
SELECT sp.package,
       sp.repo,
       sp.name                                                            AS soname,
       sp.ver                                                             AS sover,
       "substring"(pi.filename, "position"(pi.filename, '.so'::text) + 3) AS sodepver,
       pi.package                                                         AS dep_package,
       pi.repo                                                            AS dep_repo,
       pi.version                                                         AS dep_version
FROM pv_package_sodep sp
         JOIN v_packages_new vp USING (package, version, repo)
         JOIN pv_repos rp ON rp.name = sp.repo
         JOIN pv_repos rd
              ON (rd.architecture = rp.architecture OR rd.architecture = 'all'::text) AND rp.testing <= rd.testing AND
                 (rp.component = rd.component OR rp.component = 'main'::text)
         JOIN pv_package_issues pi ON pi.repo = rd.name AND pi.package <> sp.package AND
                                      "substring"(pi.filename, 1, "position"(pi.filename, '.so'::text) + 2) =
                                      sp.name AND
                                      (sp.ver || '.'::text) ~~ ((pi.detail ->> 'sover_provide'::text) || '.%'::text) AND
                                      pi.errno = 431 AND pi.detail IS NOT NULL
WHERE sp.depends = 0;
```

## v_so_breaks_dep

Collect reverse library dependencies, and provide reverse library dependencies of the direct reverse library dependencies, e.g.:

- package: A
- dep_package: B
- deplist: {C, D}

means:

- A provides a shared library which B depends i.e. B is a reverse library dependency of A
- C and D are a reverse library dependency of A
- B is a reverse library dependency of C and D

This info can be used to reorder reverse library dependencies upon SONAME bump.


```sql
create materialized view v_so_breaks_dep as
-- collect pairs or (package A, package B) from v_so_breaks
WITH pkg_so_breaks AS (SELECT DISTINCT v_so_breaks.package,
                                       v_so_breaks.dep_package
                       FROM v_so_breaks)
SELECT s.package,
       s.dep_package,
       COALESCE(q.deplist, ARRAY []::text[]) AS deplist
FROM pkg_so_breaks s
         LEFT JOIN (SELECT s1.package,
                           s1.dep_package,
                           array_agg(s2.dep_package) AS deplist
                    FROM pkg_so_breaks s1
                             -- find another dep_package
                             -- where current dep_package depends on
                             -- becomes deplist
                             JOIN pkg_so_breaks s2 ON s1.package = s2.package AND s1.dep_package <> s2.dep_package
                             JOIN (SELECT package_dependencies.package,
                                          package_dependencies.dependency
                                   FROM package_dependencies
                                   WHERE package_dependencies.relationship = ANY
                                         (ARRAY ['PKGDEP'::text, 'BUILDDEP'::text])
                                   UNION
                                   SELECT pkg_so_breaks.dep_package AS package,
                                          pkg_so_breaks.package     AS dependency
                                   FROM pkg_so_breaks) d ON d.dependency = s1.dep_package AND d.package = s2.dep_package
                    GROUP BY s1.package, s1.dep_package) q USING (package, dep_package);
```
