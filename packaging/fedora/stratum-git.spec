%global app_id io.github.stratum_git.Stratum
%global debug_package %{nil}

Name:           stratum-git
Version:        0.1.0
Release:        4%{?dist}
Summary:        Native Git GUI client for Linux

License:        GPL-3.0-or-later
URL:            https://example.invalid/stratum
# The source tarball is produced by scripts/make-source-tarball.sh and embeds
# the vendored Rust dependencies, so the build works without network access.
Source0:        %{name}-%{version}.tar.gz

BuildRequires:  rust >= 1.85
BuildRequires:  cargo
BuildRequires:  gtk4-devel >= 4.12
BuildRequires:  libadwaita-devel >= 1.4
BuildRequires:  desktop-file-utils
BuildRequires:  libappstream-glib
Requires:       git
Requires:       gtk4%{?_isa} >= 4.12
Requires:       libadwaita%{?_isa} >= 1.4

%description
Stratum is a native Git client for Linux built with GTK4 and libadwaita.
It provides a changes view with hunk-level staging, a unified and
side-by-side diff viewer, a graphical commit graph, branch, tag, stash and
remote management, conflict resolution, and graphical clone and init
workflows. Authentication, signing and credential handling are delegated to
the standard Git tooling (SSH agent, credential helpers, known hosts).

%prep
%autosetup -n %{name}-%{version}

%build
export CARGO_HOME=%{_builddir}/cargo-home
export CARGO_NET_OFFLINE=true
cargo build --release --offline

%install
install -Dpm 0755 target/release/stratum %{buildroot}%{_bindir}/stratum
install -Dpm 0755 target/release/stratum-askpass %{buildroot}%{_bindir}/stratum-askpass
install -Dpm 0644 data/%{app_id}.desktop %{buildroot}%{_datadir}/applications/%{app_id}.desktop
install -Dpm 0644 data/%{app_id}.metainfo.xml %{buildroot}%{_metainfodir}/%{app_id}.metainfo.xml
install -Dpm 0644 data/icons/hicolor/scalable/apps/%{app_id}.svg \
    %{buildroot}%{_datadir}/icons/hicolor/scalable/apps/%{app_id}.svg

%check
desktop-file-validate %{buildroot}%{_datadir}/applications/%{app_id}.desktop
appstream-util validate-relax --nonet %{buildroot}%{_metainfodir}/%{app_id}.metainfo.xml

%post
gtk-update-icon-cache -f -t %{_datadir}/icons/hicolor &>/dev/null || :
update-desktop-database %{_datadir}/applications &>/dev/null || :

%postun
gtk-update-icon-cache -f -t %{_datadir}/icons/hicolor &>/dev/null || :
update-desktop-database %{_datadir}/applications &>/dev/null || :

%files
%license LICENSE
%doc README.md CHANGELOG.md
%{_bindir}/stratum
%{_bindir}/stratum-askpass
%{_datadir}/applications/%{app_id}.desktop
%{_metainfodir}/%{app_id}.metainfo.xml
%{_datadir}/icons/hicolor/scalable/apps/%{app_id}.svg

%changelog
* Fri Oct 09 2026 Stratum contributors <maintainers@example.invalid> - 0.1.0-4
- Sidebar: application name and small Initialize/Open/Clone icons in the
  header; removed the large sync button and the large Initialize button
- Fixed the Recent repositories list
- Fetch is now a direct action (Ctrl+F); the Remotes tab keeps the options
  dialog
- Ctrl+1..6 switch between Changes, History, Branches, Tags, Stashes and
  Remotes
- Automatic refresh of the active repository every few seconds
- Selecting a change can be toggled off; multi-selection survives refreshes
- Authentication cancellation is reported clearly instead of as an auth error
- The askpass dialog can add the key to ssh-agent for the session
- History search can match changed file names (case-insensitive substring)
- Keyboard shortcuts are listed and editable in Settings

* Wed Oct 07 2026 Stratum contributors <maintainers@example.invalid> - 0.1.0-3
- Fix the askpass dialog: the password field now takes focus immediately and
  Enter submits the answer; an empty submission is no longer ignored silently
- Kill the whole git/ssh/askpass process group on cancel so progress actually
  stops and dialogs close
- Never let a lingering askpass process hold the output pipes open: operations
  can no longer hang and the progress bar always disappears
- Remove a spurious GTK header-bar warning from the askpass dialog

* Wed Oct 07 2026 Stratum contributors <maintainers@example.invalid> - 0.1.0-2
- Add the native askpass helper so passphrase-protected SSH keys and HTTPS
  credentials can be entered in a GUI dialog
- One-click pull/push plus a context-aware Sync button with ahead/behind counts

* Wed Oct 07 2026 Stratum contributors <maintainers@example.invalid> - 0.1.0-1
- Initial release
