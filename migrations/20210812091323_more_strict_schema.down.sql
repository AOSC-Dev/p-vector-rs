BEGIN;
ALTER TABLE pv_packages ALTER COLUMN section DROP DEFAULT;
ALTER TABLE pv_packages ALTER COLUMN section DROP NOT NULL;
ALTER TABLE pv_package_dependencies ALTER COLUMN value DROP NOT NULL;
COMMIT;
