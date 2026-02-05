Name:           moltis
Version:        0.1.0
Release:        1%{?dist}
Summary:        Personal AI gateway inspired by OpenClaw
License:        MIT
URL:            https://www.moltis.org/

%description
Moltis is a personal AI gateway inspired by OpenClaw. One binary, multiple LLM providers.

%install
mkdir -p %{buildroot}%{_bindir}
install -m 755 %{_sourcedir}/moltis %{buildroot}%{_bindir}/moltis

%files
%{_bindir}/moltis
