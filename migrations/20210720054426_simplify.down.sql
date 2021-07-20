-- Revert changes
BEGIN;
ALTER TABLE pv_repos ADD COLUMN IF NOT EXISTS realname TEXT NOT NULL;
UPDATE pv_repos SET realname = split_part(name, '/', 1);
ALTER TABLE pv_package_files ALTER COLUMN ftype
SET DATA TYPE TEXT USING
CASE ftype 
    WHEN 0 THEN 'reg'
    WHEN 1 THEN 'lnk'
    WHEN 2 THEN 'lnk'
    WHEN 3 THEN 'chr'
    WHEN 4 THEN 'blk'
    WHEN 5 THEN 'dir'
    WHEN 6 THEN 'fifo'
    ELSE NULL
END;
COMMIT;
