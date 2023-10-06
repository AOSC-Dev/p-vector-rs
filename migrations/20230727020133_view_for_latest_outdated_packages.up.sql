-- Add views to query latest packages and outdated packages

CREATE OR REPLACE VIEW v_packages_ordered AS
(
SELECT package,
       repo,
       version,
       architecture,
       rank() OVER (PARTITION BY package, repo, architecture ORDER BY _vercomp DESC) AS pos
FROM pv_packages
    );

CREATE OR REPLACE VIEW v_packages_latest AS
(
SELECT package,
       repo,
       version,
       architecture
FROM v_packages_ordered
WHERE pos = 1
    );

CREATE OR REPLACE VIEW v_packages_outdated AS
(
SELECT package,
       repo,
       version,
       architecture
FROM v_packages_ordered
WHERE pos > 1
    );
