-- Add namespace column to git_accounts for remote-repo listing scope.
ALTER TABLE git_accounts ADD COLUMN namespace TEXT;
