-- Simplify the database
BEGIN;
-- delete realname column
ALTER TABLE pv_repos DROP COLUMN IF EXISTS realname;
-- change ftype to an integer
ALTER TABLE pv_package_files ALTER COLUMN ftype
SET DATA TYPE SMALLINT USING
CASE ftype 
    WHEN 'reg' THEN 0
    WHEN 'lnk' THEN 2
    WHEN 'chr' THEN 3
    WHEN 'blk' THEN 4
    WHEN 'dir' THEN 5
    WHEN 'fifo' THEN 6
    ELSE NULL
END;
COMMIT;
