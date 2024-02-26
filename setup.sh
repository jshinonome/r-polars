dnf upgrade --refresh -y
dnf install epel-release -y
dnf config-manager --set-enabled crb
dnf install R git cmake -y
dnf install libcurl-devel openssl-devel harfbuzz-devel fribidi-devel freetype-devel libpng-devel libjpeg-turbo-devel libxml2-devel gnutls-devel fontconfig-devel freetype-devel libtiff-devel -y
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
sh -c "$(curl --location https://taskfile.dev/install.sh)" -- -d -b ~/.local/bin

sudo dnf install https://dl.fedoraproject.org/pub/epel/epel-release-latest-9.noarch.rpm

export R_VERSION=4.3.1
curl -O https://cdn.rstudio.com/r/rhel-9/pkgs/R-${R_VERSION}-1-1.x86_64.rpm
dnf install R-${R_VERSION}-1-1.x86_64.rpm -y
ln -s /opt/R/${R_VERSION}/bin/R /usr/local/bin/R
ln -s /opt/R/${R_VERSION}/bin/Rscript /usr/local/bin/Rscript
# install.packages("devtools")
