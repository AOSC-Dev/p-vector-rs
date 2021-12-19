-- Fix incorrectly label so deps
BEGIN;
UPDATE pv_package_sodep SET name = name || '.so' WHERE name NOT LIKE '%.so';
COMMIT;
