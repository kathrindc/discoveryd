USE discoveryd;

CREATE TABLE domains (
    `id` INT(11) NOT NULL AUTO_INCREMENT,
    `domain` VARCHAR(120) NOT NULL,
    `imap_server` VARCHAR(120) NOT NULL,
    `imap_port` INT(6) NOT NULL,
    `imap_ssl` BOOLEAN NOT NULL,
    `smtp_server` VARCHAR(120) NOT NULL,
    `smtp_port` INT(6) NOT NULL,
    `smtp_ssl` BOOLEAN NOT NULL,
    `activesync_url` TEXT,
    `activesync_preferred` BOOLEAN NOT NULL DEFAULT 0,
    `sts_mode` VARCHAR(20) NOT NULL,
    PRIMARY KEY (`id`),
    UNIQUE (`domain`)
) ENGINE=InnoDB DEFAULT CHARSET=UTF8;

CREATE TABLE mx_whitelists (
    `id` INT(11) NOT NULL AUTO_INCREMENT,
    `domain_id` INT(11) NOT NULL,
    `host` VARCHAR(120) NOT NULL,
    PRIMARY KEY (`id`),
    FOREIGN KEY (`domain_id`) REFERENCES domains (`id`)
) ENGINE=InnoDB DEFAULT CHARSET=UTF8;