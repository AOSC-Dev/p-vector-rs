# P-Vector Setup Guide

Welcome to the P-Vector setup guide. This guide will help you setup P-Vector and host your own APT/dpkg software repository.

# Installation

P-Vector is only intended to run on modern Linux with Rust support, and strong processing and storage performance are advised for efficient usage.

If you are using AOSC OS, simply install `p-vector` from the repository by executing `sudo apt-get install p-vector`. If you are using on other Linux distributions, please refer to the [Build Instructions](https://github.com/AOSC-Dev/p-vector-rs#building-instructions).

# Setup Guide

P-Vector is designed with ease of deployment in mind. However, it still requires a few steps due to the complexity of the APT repository layout.

## Directory Structure

P-Vector expects an APT repository layout according to a defined directory structure. A typical APT repository layout may look as follows:

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

This may seem daunting at the first glance, but it's really simpler than it seems.

First, create two directories, `pool` and `dists`, under a directory of your choice. This directory is preferably on a large capacity storage if you have a large number of packages.

Next, you need to decide on the branch and component names. If you are having difficulties coming up with a name, naming your branch as `stable` and your component as `main` is conventional practice. In which case, assuming you are currently inside the chosen directory, you just need to run `mkdir -pv {pool,dists}/stable/main/`.

Finally, you will need to move your packages into `pool/stable/main/`. If you want to have more branches, you just need to create more directories under the `pool` directory, and P-Vector will take care of the rest.

## Configuration

When you are done with moving your packages, it's time to configure your P-Vector instance.

Depending on how you want to use your repository (whether it's private or public), the configuration varies. This guide will not cover all possible configurations, but focusing on the most common ones.

### Setting up the database

P-Vector relies heavily on the PostgreSQL database. You should have already installed it in your installation step, if not, please revisit the installation instructions.

Before doing anything, you need to make sure PostgreSQL server is running. On systemd-based systems, `sudo systemctl start postgresqld` or `sudo systemctl start postgresql` should do the job. If you are having trouble with starting up PostgreSQL server, you may want to consult the documentation provided by your Linux distribution on how to launch and initialize the PostgreSQL server.

It's recommended that you use a separate database for P-Vector data. You can create a new database under your own user account using `createdb <database name>`.

If you plan to run P-Vector using a different user, please use `su` or other commands (such as `doas`) to switch to that user and then create a corresponding PostgreSQL user/role in the database like so: `createuser --interactive <username>`. Follow the on-screen instructions to finish creating the user/role. Then, you need to create the database on _that user's behalf_ with `createdb -O <username> <database name>`.

If you encountered any issue with permissions, you might need to switch to a user with PostgreSQL superuser permissions (note: it's **NOT** `root`!). This user is usually named `postgres`. If not, please consult the documentation of your Linux distribution on which account is the PostgreSQL superuser account. When using that user, you may want to run commands in this fashion: `sudo -u postgres <command>`.

### Creating P-Vector configuration

After setting up your database, now it's time to create your P-Vector configuration. A template is provided in this repository.

The configuration template is heavily commented, if you have used the older version of P-Vector, you can quickly migrate from your old settings. If you are new to this, see below for some common senarios and settings.

#### General settings

- `db_pgconn`: This is the database connection setting, you would need to set it in this format: `postgresql://localhost/<database name>`. For example: `db_pgconn = "postgresql://localhost/packages"` means connecting to a database named `packages`. If you need more advanced configuration, please see https://www.postgresql.org/docs/13/libpq-connect.html#LIBPQ-CONNSTRING.
- `path`: This is the path to the root of your repository. This is the directory containing both `pool` and `dists`.
- `origin`: Branding name of your repository.
- `label`: Label of your repository.
- `codename`: Codename of your repository.
- `ttl`: This is the forced refresh interval of your repository. The value is in days, it indicates how often the repository metadata is refreshed when there is no activity in the repository.
- `certificate`: This is the certificate used for signing your repository. If you don't have one, skip this setting for now and read the following sections carefully.

#### Public repository (non-AOSC)

- Set `change_notifier = null`.
- Set `abbs_sync = false`.
- Optionally set `qa_interval = -1` if you don't want or can't analyze the QA inspection data in the database.
- `[[branch]]` sections: At least set the "main" branch information of your repository.

#### Private repository (non-AOSC)

- Set `change_notifier = null`.
- Set `abbs_sync = false`.
- Optionally set `qa_interval = -1` if you don't want or can't analyze the QA inspection data in the database.
- `[[branch]]` sections: Optionally set the "main" branch information of your repository.

### Signing your repository

If you skipped the `certificate` setting in the previous section, you need to pay attention to this section.

APT repository needs to be signed in order to verify the integrity of the files in your repository. APT expects the signatures in the OpenPGP format.

P-Vector can handle OpenPGP signing or delegate the signing process to an external provider in senarios where it is necessary. If you already have an existing key for signing, please refer to the "Using an existing key from ..." sections below.

#### Generating a new certificate using P-Vector

You can use P-Vector to generate a certificate if you don't want to use `gpg` or don't know how to use it.
<p>

1. Run `p-vector gen-key` and follow the on-screen instructions.
1. Make sure that the private key is stored in a safe location.
1. Edit the `certificate` setting in your configuration file according to the instructions shown in step 1.
1. You are good to go!

</p>

#### Using an existing key from local files

If you plan to use an existing key for signing, please make sure P-Vector supports the algorithm of your signing key.

<details>
<summary>Click here to see the support matrix</summary>
<p>

The following general public key algorithms are supported:

- RSA
- DSA
- ECC

When using ECC signing, the following signing algorithms are supported:

- ECDSA
- EdDSA

When using ECC signing, the following elliptic curves are supported:

- Ed25519 (when using EdDSA)
- Cv25519 (X25519/Curve25519)
- Brainpool P-256
- Brainpool P-512
- NIST P-256
- NIST P-384
- NIST P-521
</p>
</details>

#### Using an existing key from the GnuPG keystore
<p>

1. Export the public key of the key of your choice by running `gpg --export <fingerprint> > pubkey.pgp`.
1. [Optional] Move the file `pubkey.pgp` to a public location so that your users could download it.
1. Edit the `certificate` setting in your configuration file into something like this: `certificate = "gpg:///path/to/pubkey.pgp"`.
1. Make sure `gpg-agent` is up and running. Please note that `gpg-agent` is **user-specific**: if you want to run `p-vector` using a different user, you need to make sure `gpg-agent` is launched as that user as well and that `gpg-agent` could access your private key from that account
1. You are good to go!

</p>

#### Using an existing key from hardware security tokens (e.g. Yubikey, Nitrokey)
<p>

1. Fetch the public key from your hardware security token using `gpg --card-status`. This step will also generate a key stub on your local storage.
1. Take note of the fingerprint of the key in the signing slot of your smartcard/hardware security token. This is the long string of letters and numbers after `Signature key ....:` row displayed in step 1.
1. Export the public key of the key from step 2 by running `gpg --export <fingerprint> > pubkey.pgp`.
1. Edit the `certificate` setting in your configuration file into something like this: `certificate = "gpg:///path/to/pubkey.pgp"`.
1. **NOTE**: Signing will take place on the smartcard/hardware security token's onboard processor, so you need to plug it in when running P-Vector. Some hardware security tokens require that you to perform a specific action when signing like touching/tapping the button on your token or scanning your fingerprint on the scanner. This means that P-Vector will **NOT** be able to work unattended.
1. Make sure `gpg-agent` is up and running. Please note that `gpg-agent` is **user-specific**: if you want to run `p-vector` using a different user, you need to make sure `gpg-agent` is launched as that user as well and that `gpg-agent` could access your private key from that account
1. You are good to go!

</p>

### Test drive

Congratulations! You have successfully setup your P-Vector instance. Now you can run `p-vector -c <path/to/configuration/file> full` to see it in action. The first run may be slow depending on the amount and size of packages, but it will be much faster in subsequent runs.
