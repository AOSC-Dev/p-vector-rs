-- Tables
CREATE TABLE IF NOT EXISTS pv_repos (
    name TEXT PRIMARY KEY, -- key: bsp-sunxi-armel/testing
    realname TEXT NOT NULL,     -- group key: amd64, bsp-sunxi-armel
    path TEXT NOT NULL,         -- testing/main
    testing INTEGER NOT NULL,   -- 0, 1, 2
    branch TEXT NOT NULL,       -- stable, testing, explosive
    component TEXT NOT NULL,    -- main, bsp-sunxi, opt-avx2
    architecture TEXT NOT NULL,  -- amd64, all
    mtime TIMESTAMP WITH TIME ZONE NOT NULL
);

CREATE TABLE IF NOT EXISTS pv_packages (
    package TEXT NOT NULL,
    version TEXT NOT NULL,
    repo TEXT NOT NULL,
    architecture TEXT NOT NULL,
    filename TEXT NOT NULL,
    size BIGINT NOT NULL,
    sha256 TEXT NOT NULL,
    mtime INTEGER NOT NULL,
    debtime INTEGER NOT NULL,
    section TEXT NOT NULL,
    installed_size BIGINT NOT NULL,  -- x1024
    maintainer TEXT NOT NULL,
    description TEXT NOT NULL,
    _vercomp TEXT NOT NULL,
    PRIMARY KEY (package, version, repo),
    CONSTRAINT fkey_repo FOREIGN KEY (repo)
    REFERENCES pv_repos (name) ON DELETE CASCADE INITIALLY DEFERRED
);

UPDATE pv_repos r SET mtime=(SELECT to_timestamp(max(mtime)) FROM pv_packages p WHERE p.repo=r.name) WHERE mtime IS NULL;

CREATE TABLE IF NOT EXISTS pv_package_duplicate (
    package TEXT NOT NULL,
    version TEXT NOT NULL,
    repo TEXT NOT NULL,
    architecture TEXT NOT NULL,
    filename TEXT NOT NULL,
    size BIGINT NOT NULL,
    sha256 TEXT NOT NULL,
    mtime INTEGER NOT NULL,
    debtime INTEGER NOT NULL,
    section TEXT NOT NULL,
    installed_size BIGINT NOT NULL,  -- x1024
    maintainer TEXT NOT NULL,
    description TEXT NOT NULL,
    _vercomp TEXT NOT NULL,
    PRIMARY KEY (filename),
    CONSTRAINT fkey_package FOREIGN KEY (package, version, repo)
    REFERENCES pv_packages (package, version, repo) ON DELETE CASCADE INITIALLY DEFERRED
);

CREATE TABLE IF NOT EXISTS pv_package_dependencies (
    package TEXT,
    version TEXT,
    repo TEXT,
    relationship TEXT,
    value TEXT,
    PRIMARY KEY (package, version, repo, relationship),
    CONSTRAINT fkey_package FOREIGN KEY (package, version, repo)
    REFERENCES pv_packages (package, version, repo) ON DELETE CASCADE INITIALLY DEFERRED
);

CREATE TABLE IF NOT EXISTS pv_package_sodep (
    package TEXT,
    version TEXT,
    repo TEXT,
    depends INTEGER, -- 0 provides, 1 depends
    name TEXT,
    ver TEXT,
    CONSTRAINT fkey_package FOREIGN KEY (package, version, repo)
    REFERENCES pv_packages (package, version, repo) ON DELETE CASCADE INITIALLY DEFERRED 
    -- PRIMARY KEY (package, version, repo, depends, name)
);

CREATE TABLE IF NOT EXISTS pv_package_files (
    package TEXT,
    version TEXT,
    repo TEXT,
    path TEXT,
    name TEXT,
    size BIGINT,
    ftype TEXT,
    perm INTEGER,
    uid BIGINT,
    gid BIGINT,
    uname TEXT,
    gname TEXT,
    CONSTRAINT fkey_package FOREIGN KEY (package, version, repo)
    REFERENCES pv_packages (package, version, repo) ON DELETE CASCADE INITIALLY DEFERRED
    -- PRIMARY KEY (package, version, repo, path, name)
);

CREATE TABLE IF NOT EXISTS pv_package_issues (
    id SERIAL PRIMARY KEY,
    package TEXT,
    version TEXT,
    repo TEXT,
    errno INTEGER,
    level SMALLINT,
    filename TEXT,
    ctime TIMESTAMP WITH TIME ZONE DEFAULT (now()),
    mtime TIMESTAMP WITH TIME ZONE DEFAULT (now()),
    atime TIMESTAMP WITH TIME ZONE DEFAULT (now()),
    detail JSONB,
    UNIQUE (package, version, repo, errno, filename)
);

CREATE TABLE IF NOT EXISTS pv_issues_stats (
    repo TEXT,
    errno INTEGER,
    cnt INTEGER,
    total INTEGER,
    updated TIMESTAMP WITH TIME ZONE DEFAULT (now())
);

CREATE MATERIALIZED VIEW IF NOT EXISTS v_packages_new AS 
    SELECT DISTINCT ON (repo, package) package, version, repo, 
        architecture, filename, size, sha256, mtime, debtime, 
        section, installed_size, maintainer, description, _vercomp 
    FROM pv_packages 
    WHERE debtime IS NOT NULL 
    ORDER BY repo, package, _vercomp DESC;

-- Indices
CREATE INDEX IF NOT EXISTS idx_pv_repos_path ON pv_repos (path, architecture);
CREATE INDEX IF NOT EXISTS idx_pv_repos_architecture ON pv_repos (architecture, testing);
CREATE INDEX IF NOT EXISTS idx_pv_packages_repo ON pv_packages (repo);
CREATE INDEX IF NOT EXISTS idx_pv_package_duplicate_package ON pv_package_duplicate (package, version, repo);
CREATE INDEX IF NOT EXISTS idx_pv_package_issues_errno ON pv_package_issues (errno);
CREATE INDEX IF NOT EXISTS idx_pv_package_issues_mtime  ON pv_package_issues USING brin (mtime);
CREATE INDEX IF NOT EXISTS idx_pv_package_issues_atime ON pv_package_issues (atime);
CREATE UNIQUE INDEX IF NOT EXISTS idx_pv_issues_stats_pkey ON pv_issues_stats (repo, errno, updated DESC);

-- Views
CREATE MATERIALIZED VIEW IF NOT EXISTS v_dpkg_dependencies AS
SELECT package, version, repo, relationship, nr,
  depspl[1] deppkg, depspl[2] deparch, depspl[3] relop, depspl[4] depver,
  comparable_dpkgver(depspl[4]) depvercomp
FROM (
  SELECT package, version, repo, relationship, nr, regexp_match(dep,
    '^\s*([a-zA-Z0-9.+-]{2,})(?::([a-zA-Z0-9][a-zA-Z0-9-]*))?' ||
    '(?:\s*\(\s*([>=<]+)\s*([0-9a-zA-Z:+~.-]+)\s*\))?(?:\s*\[[\s!\w-]+\])?' ||
    '\s*(?:<.+>)?\s*$') depspl
  FROM (
    SELECT package, version, repo, relationship, nr,
      unnest(string_to_array(dep, '|')) dep
    FROM (
      SELECT d.package, d.version, d.repo, d.relationship, v.nr, v.val dep
      FROM pv_package_dependencies d
      INNER JOIN v_packages_new n USING (package, version, repo)
      INNER JOIN LATERAL unnest(string_to_array(d.value, ','))
        WITH ORDINALITY AS v(val, nr) ON TRUE
    ) q1
  ) q2
) q3;

CREATE MATERIALIZED VIEW IF NOT EXISTS v_so_breaks AS
SELECT sp.package, sp.repo, sp.name soname, sp.ver sover, sd.ver sodepver,
  sd.package dep_package, sd.repo dep_repo, sd.version dep_version
FROM pv_package_sodep sp
INNER JOIN v_packages_new vp USING (package, version, repo)
INNER JOIN pv_repos rp ON rp.name=sp.repo
INNER JOIN pv_repos rd ON rd.architecture IN (rp.architecture, 'all')
AND rp.testing<=rd.testing AND rp.component IN (rd.component, 'main')
INNER JOIN pv_package_sodep sd ON sd.depends=1
AND sd.repo=rd.name AND sd.name=sp.name AND sd.package!=sp.package
AND (sp.ver=sd.ver OR sp.ver LIKE sd.ver || '.%')
INNER JOIN v_packages_new vp2
ON vp2.package=sd.package AND vp2.version=sd.version AND vp2.repo=sd.repo
WHERE sp.depends=0
UNION ALL
SELECT sp.package, sp.repo, sp.name soname, sp.ver sover,
  substring(pi.filename from position('.so' in pi.filename)+3) sodepver,
  pi.package dep_package, pi.repo dep_repo, pi.version dep_version
FROM pv_package_sodep sp
INNER JOIN v_packages_new vp USING (package, version, repo)
INNER JOIN pv_repos rp ON rp.name=sp.repo
INNER JOIN pv_repos rd ON rd.architecture IN (rp.architecture, 'all')
AND rp.testing<=rd.testing AND rp.component IN (rd.component, 'main')
INNER JOIN pv_package_issues pi
ON pi.repo=rd.name AND pi.package!=sp.package
AND substring(pi.filename from 1 for position('.so' in pi.filename)+2)=sp.name
AND (sp.ver || '.') LIKE (detail->>'sover_provide') || '.%'
AND pi.errno=431 AND pi.detail IS NOT NULL
WHERE sp.depends=0;

CREATE MATERIALIZED VIEW IF NOT EXISTS v_so_breaks_dep AS
WITH pkg_so_breaks AS (SELECT DISTINCT package, dep_package FROM v_so_breaks)
SELECT s.package, s.dep_package, coalesce(deplist, array[]::text[]) deplist
FROM pkg_so_breaks s
LEFT JOIN (
  SELECT s1.package, s1.dep_package, array_agg(s2.dep_package) deplist
  FROM pkg_so_breaks s1
  INNER JOIN pkg_so_breaks s2
  ON s1.package=s2.package AND s1.dep_package!=s2.dep_package
  INNER JOIN (
    SELECT package, dependency
    FROM package_dependencies
    WHERE relationship IN ('PKGDEP', 'BUILDDEP')
    UNION
    SELECT dep_package package, package dependency FROM pkg_so_breaks
  ) d
  ON d.dependency=s1.dep_package AND d.package=s2.dep_package
  GROUP BY s1.package, s1.dep_package
) q USING (package, dep_package);

CREATE INDEX IF NOT EXISTS idx_pv_repos_mtime ON pv_repos (mtime);
CREATE INDEX IF NOT EXISTS idx_pv_packages_mtime ON pv_packages (mtime);
CREATE INDEX IF NOT EXISTS idx_pv_package_sodep_package ON pv_package_sodep (package, version, repo);
CREATE INDEX IF NOT EXISTS idx_pv_package_sodep_name ON pv_package_sodep (name, repo) WHERE depends=0;
CREATE INDEX IF NOT EXISTS idx_pv_package_files_package ON pv_package_files (package, version, repo);
CREATE INDEX IF NOT EXISTS idx_pv_package_files_path ON pv_package_files (path);
CREATE INDEX IF NOT EXISTS idx_pv_package_files_name ON pv_package_files (name);
CREATE INDEX IF NOT EXISTS idx_v_packages_new_package ON v_packages_new (package, repo, version);
CREATE INDEX IF NOT EXISTS idx_v_packages_new_mtime ON v_packages_new (mtime);
CREATE INDEX IF NOT EXISTS idx_v_dpkg_dependencies_package ON v_dpkg_dependencies (package, version, repo);
CREATE INDEX IF NOT EXISTS idx_v_dpkg_dependencies_dep ON v_dpkg_dependencies (relationship, deppkg, depvercomp);
CREATE INDEX IF NOT EXISTS idx_v_so_breaks_package ON v_so_breaks (package, repo);
CREATE INDEX IF NOT EXISTS idx_v_so_breaks_dep_package ON v_so_breaks (dep_package, dep_repo, dep_version);
CREATE INDEX IF NOT EXISTS idx_v_so_breaks_dep ON v_so_breaks_dep (package);

-- synchronized Tables
CREATE TABLE IF NOT EXISTS pv_dbsync (
  name TEXT PRIMARY KEY,
  etag TEXT,
  updated TIMESTAMP WITH TIME ZONE DEFAULT (now())
);
