# P-Vector Setup Guide

Welcome to P-Vector Setup Guide. This guide will help you setup P-Vector and host your own dpkg software repository.

# Installation

P-Vector is only intended to run on modern Linux. It might work on other platforms, but that is not guaranteed.

If you are using AOSC OS, you just need to install `p-vector` from the repository by running `sudo apt-get install p-vector`. If you are using on other Linux distributions, please see the [Build Instructions](https://github.com/AOSC-Dev/p-vector-rs#building-instructions).

# Setup Guide

P-Vector aims to be as easily setup as possible. However, it still requires some setup steps due to the complexity of the APT repository layout.

## Directory Structure

To make P-Vector work properly, you first need to organize your dpkg repository according to a defined directory structure.

A typical dpkg repository layout may look as follows:

```
/
|- pool/
    |- stable/
        |- main/
        |- <... other components>
    |- testing/
        |- main/
        |- bsp-sunxi/
        |- <... other components>
    |- <... other branches>
|- dists/
    |- stable/
        |- main/
        |- <... other components>
    |- testing/
        |- main/
        |- bsp-sunxi/
        |- <... other components>
    |- <... other branches>
```

It may seem very daunting at the first glance. But what you really need to do to start is to create two directories `pool` and `dists` under a directory of your choice.
This directory is preferably on a large capacity storage if you have a large number of packages.
Then you need to decide on the branch and component names. If you are having difficulties coming up with a name, naming your branch as `stable` and your component as `main` will be enough. In which case, assuming you are currently inside the chosen directory, you just need to run `mkdir -p pool/stable/main/` and `mkdir -p dists/stable/main`.
After that, you will need to move your files into `pool/stable/main/`. If you want to have more branches, you just need to create more directories under  the `pool` directory.

## Configuration

When you are done with moving your files, it's time to configure your P-Vector instance.

Depending on how you want to use your repository (whether it's private or public), the configuration varies. This guide will not cover all possible configurations, but focusing on the most common ones.

### Setting up the database

First of all, P-Vector relies heavily on the PostgreSQL database. You should have already installed it in your installation step, if not, please re-visit the installation step to install it.

Before doing anything, you need to make sure PostgreSQL server is running. On modern systems, run `sudo systemctl start postgresqld` or `sudo systemctl start postgresql` should do it. If you are having troubles with starting up PostgreSQL server, you may want to consult the documentation provided by your Linux distribution on how to launch and initialize the PostgreSQL server.

It's recommended that you use a separate database for P-Vector data. You can create a new database under your own account using `createdb <database name>`.

If you plan to run P-Vector using a different account, please use `sudo` or other commands (such as `doas`) to switch to that account and then create a corresponding PostgreSQL user/role in the database like so: `createuser --interactive <username>`. Follow the on-screen instructions to finish creating the user/role. Then, you need to create the database on _that user's behalf_ like this: `createdb -O <username> <database name>`.

If you encountered any permission problems, you might need to switch to an account with PostgreSQL superuser permissions (note: it's **NOT** the `root` account!), this account is usually named `postgres`. If not, please check the documentation of your Linux distribution on which account is the PostgreSQL superuser account. When using that account, you may want to run the command like this: `sudo -u postgres <command>`.

### Writing configuration file

After setting up your database, now it's time to write the configuration. A template is provided in this repository, and you can use it as a starting point.

The configuration file is heavily commented, if you have used the older version of p-vector, you can quickly migrate from the old settings. If you are new to this, see below for some common senario and settings.

#### General settings

- `db_pgconn`: this is the database connection settings, you need to set it in this format: `postgresql://localhost/<database name>`. For example: `db_pgconn = "postgresql://localhost/packages"` means connecting to a database named `packages`. If you need more advanced configuration, see https://www.postgresql.org/docs/13/libpq-connect.html#LIBPQ-CONNSTRING.
- `path`: this is the path to the root of your repository folder. This is the directory containing both `pool` and `dists`.
- `origin`: brand name of your repository.
- `label`: label name of your repository.
- `codename`: code name of your repository.
- `ttl`: this is the forced refresh interval of your repository. The value is in days, it indicates how often the repository metadata is refreshed when there is no activity in the repository.
- `certificate`: this is the signing certificate for your repository. If you don't have one, skip this setting for now and see the following sections carefully.

#### Public repository (non-AOSC)

- set `change_notifier = null`
- set `abbs_sync = false`
- optionally set `qa_interval = -1` if you don't want or can't analyze the QA inspection data in the database
- `[[branch]]` sections: at least set the "main" branch information of your repository

#### Private repository (non-AOSC)

- set `change_notifier = null`
- set `abbs_sync = false`
- optionally set `qa_interval = -1` if you don't want or can't analyze the QA inspection data in the database
- `[[branch]]` sections: optionally set the "main" branch information of your repository

### Repository signing

If you skipped the `certificate` setting in the previous section, you need to pay attention to this section.

APT repository needs to be signed in order to verify the integrity of the files in your repository. APT expect the signatures in the OpenPGP format.
P-Vector currently can handle OpenPGP signing itself or delegate the signing process to an external provider in senarios where it is necessary.

If you don't want to use `gpg` or don't know how to use it, you can **use P-Vector to generate the certificate**:
<details>
<summary>Click here to see the instructions for generating a certificate using P-Vector</summary>
<p>

1. Run `p-vector gen-key` and follow the on-screen instructions
1. Make sure the private key is stored in a safe location
1. Edit `certificate` setting in your configuration file according to the instructions shown in step 1
1. You are good to go!
</p>
</details>

If you want to use an **existing key from your GnuPG keystore**, you can use these instructions:
<details>
<summary>Click here to see the instructions for how to use a key from GnuPG</summary>
<p>

1. Export the public key of the key of your choice by running `gpg --export <fingerprint> > pubkey.pgp`
1. [Optional] Move the file `pubkey.pgp` to a public location so that your users could download it
1. Edit `certificate` setting in your configuration file like this: `certificate = "gpg:///path/to/pubkey.pgp"`
1. Make sure `gpg-agent` is up and running. Please note that `gpg-agent` is **user-specific**: if you want to run `p-vector` using a different user, you need to make sure `gpg-agent` is launched as that user as well and `gpg-agent` could access your private key from that account
1. You are good to go!
</p>
</details>

If you want to use an **existing key from your smartcard or hardware security tokens** (e.g. Yubikey, Nitrokey, etc), you can use these instructions:
<details>
<summary>Click here to see the instructions for how to use the key from a smartcard/hardware security token</summary>
<p>

1. Fetch the public key from your hardware security token using `gpg --card-status`. This step will also generate a key stub on your local storage
1. Note down the fingerprint of the key in the signing slot of your smartcard/hardware security token: the long string of letters and numbers after `Signature key ....:` row displayed in step 1.
1. Export the public key of the key from step 2 by running `gpg --export <fingerprint> > pubkey.pgp`.
1. Edit `certificate` setting in your configuration file like this: `certificate = "gpg:///path/to/pubkey.pgp"`.
1. **NOTE**: signing will happen on the smartcard/hardware security token's onboard processor, so you need to plug it in when running P-Vector. Some hardware security tokens require you to perform a specific action when signing like touching/tapping the button on your token or scanning your fingerprint on the scanner. This means that P-Vector will **NOT** be able to work unattended.
1. Make sure `gpg-agent` is up and running. Please note that `gpg-agent` is **user-specific**: if you want to run `p-vector` using a different user, you need to make sure `gpg-agent` is launched as that user as well and `gpg-agent` could access your private key from that account
1. You are good to go!
</p>
</details>

### Test drive

Congratulations! You have successfully setup your P-Vector. Now you can run `p-vector -c <path/to/configuration/file> full` to see it in action.
The first run will be slow, but it will be much faster in subsequent runs.
