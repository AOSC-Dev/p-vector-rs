-- Trim leading ./ and / from the path name 
BEGIN;
UPDATE pv_package_files SET path = regexp_replace(path, '^(\./|/)', '');
COMMIT;
