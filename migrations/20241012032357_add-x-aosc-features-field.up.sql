ALTER TABLE pv_packages ADD COLUMN IF NOT EXISTS features TEXT;
ALTER TABLE pv_package_duplicate ADD COLUMN IF NOT EXISTS features TEXT;
