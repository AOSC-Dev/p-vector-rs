# Database Schema

## pv_repos

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