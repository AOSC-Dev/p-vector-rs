-- Un-Fix incorrectly label so deps

-- Reverting this migration is not possible.
CREATE OR REPLACE PROCEDURE revert_not_possible() AS $$
    BEGIN
    RAISE WARNING 'Reverting migration 20211219041531_fix_so_file_records is not possible!';
    END;
$$ LANGUAGE plpgsql;

CALL revert_not_possible();
DROP PROCEDURE revert_not_possible();
