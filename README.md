# osutil

An utility to improve the workflow for working on openSUSE.

## Features

- Get outdated packages maintained by the current user

## Getting started

### Build

To build osutil:

```
$ cargo build --release
```

### Install

Copy the executable under path. For this example the folder `~/.local/bin` will be used, check that your `PATH` contains it. Feel free to use any other folder.

```
$ cp target/release/osutil ~/.local/bin
```

### Usage

Create the configuration file `$XDG_CONFIG_HOME/osutil/osutil.conf` (which defaults to `~/.config/osutil/osutil.conf`) with your build.opensuse.org credentials:

```
username = <my-username>
password = <my-password>
```

Now run the following to get the list of supported commands:

```
$ osutil
```

## FAQ

### Why add outdated command instead of using Github/RSS/Mailing Lists?

While Github supports release notifications there are many projects not hosted there.
Sourcehut supports an RSS but projects on Sourceforge and selfhosted do not. Sometimes
projects have a mailing list. Gitlab projects allows to subscribe like on Github, but it means
creating an account for many Gitlab istances. The command `outdated` uses [repology] to provide
information on outdated packages regardless of where they are hosted. The big caveat is that the
package won't be listed as outdated until at least another distribution ships the updated version.
Thanks to Arch Linux this list will almost always be up-to-date.

[repology]: https://repology.org

## License

osutil is licensed under the GPL-3.0+ license.
