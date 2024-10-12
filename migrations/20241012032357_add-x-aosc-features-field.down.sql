-- Add down migration script here
ALTER TABLE pv_packages DROP COLUMN IF EXISTS features;
