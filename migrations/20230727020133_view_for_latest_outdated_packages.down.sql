-- Revert adding views to query latest packages and outdated packages

DROP VIEW IF EXISTS v_packages_latest;
DROP VIEW IF EXISTS v_packages_outdated;
DROP VIEW IF EXISTS v_packages_ordered;
