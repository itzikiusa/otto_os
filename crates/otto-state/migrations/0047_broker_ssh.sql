-- Message Brokers: optional SSH tunnel for a Kafka cluster profile.
-- Stores the non-secret tunnel config (host/port/user/identity_file path) as a
-- JSON object; NULL = no tunnel. SSH auth is key-file/agent only, so there is
-- no secret to keep in the Keychain. Used to reach private clusters (e.g. AWS
-- MSK in a VPC) through a bastion.
ALTER TABLE broker_clusters ADD COLUMN ssh_config TEXT;
