-- More strict packages schema
BEGIN;
DELETE FROM pv_package_dependencies WHERE value = '' OR value IS NULL;
UPDATE pv_packages SET section = 'unknown' WHERE section = '' OR section IS NULL;
-- set not null
ALTER TABLE pv_packages ALTER COLUMN section SET DEFAULT 'unknown';
ALTER TABLE pv_packages ALTER COLUMN section SET NOT NULL;
ALTER TABLE pv_package_dependencies ALTER COLUMN value SET NOT NULL;
COMMIT;
