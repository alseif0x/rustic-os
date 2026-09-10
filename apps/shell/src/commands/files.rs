// SPDX-License-Identifier: Apache-2.0
use super::*;
pub(super) fn execute(s: &mut Session, a: &Args<'_>) -> Result<(), Error> {
    match argument(a, 0)? {
        "pwd" => {
            exact(a, 1)?;
            let mut b = [0; 256];
            let n = s.files.path(s.cwd, &mut b)?;
            output::bytes(&b[..n]);
            output::text("\r\n");
        }
        "cd" => {
            exact(a, 2)?;
            let id = s.files.resolve(s.cwd, argument(a, 1)?)?;
            if id != 0 && !s.files.stat(id)?.directory {
                return Err(rustic_sdk::files::Error::NotDirectory.into());
            }
            s.cwd = id;
        }
        "ls" => {
            if a.len() > 2 {
                return Err(Error::Usage);
            }
            let id = if a.len() == 2 {
                s.files.resolve(s.cwd, argument(a, 1)?)?
            } else {
                s.cwd
            };
            let mut cursor = 0;
            while let Some(m) = s.files.list(id, cursor)? {
                output::format(format_args!(
                    "{} {:4} {}\r\n",
                    if m.directory { "d" } else { "f" },
                    m.length,
                    m.name()
                ));
                cursor = m.cursor;
            }
        }
        "mkdir" | "touch" => {
            exact(a, 2)?;
            let (parent, name) = s.files.parent(s.cwd, argument(a, 1)?)?;
            s.files.create(parent, name, argument(a, 0)? == "mkdir")?;
        }
        "write" => {
            if a.len() < 3 {
                return Err(Error::Usage);
            }
            let path = argument(a, 1)?;
            let (parent, name) = s.files.parent(s.cwd, path)?;
            let mut bytes = [0; 1024];
            let mut n = 0;
            for i in 2..a.len() {
                let text = argument(a, i)?.as_bytes();
                let extra = usize::from(i > 2);
                if n + extra + text.len() > bytes.len() {
                    return Err(rustic_sdk::files::Error::Size.into());
                }
                if extra != 0 {
                    bytes[n] = b' ';
                    n += 1;
                }
                bytes[n..n + text.len()].copy_from_slice(text);
                n += text.len();
            }
            let m = match s.files.lookup(parent, name) {
                Ok(m) => m,
                Err(rustic_sdk::files::Error::NotFound) => s.files.create(parent, name, false)?,
                Err(e) => return Err(e.into()),
            };
            let m = s.files.replace(m.id, m.version, &bytes[..n])?;
            output::format(format_args!(
                "written {} bytes version={}\r\n",
                n, m.version
            ));
        }
        "cat" => {
            exact(a, 2)?;
            let id = s.files.resolve(s.cwd, argument(a, 1)?)?;
            let mut b = [0; 1024];
            let n = s.files.read(id, &mut b)?;
            output::bytes(&b[..n]);
            output::text("\r\n");
        }
        "stat" => {
            exact(a, 2)?;
            let id = s.files.resolve(s.cwd, argument(a, 1)?)?;
            let m = s.files.stat(id)?;
            output::format(format_args!(
                "id={} parent={} kind={} bytes={} version={}\r\n",
                m.id,
                m.parent,
                if m.directory { "directory" } else { "file" },
                m.length,
                m.version
            ));
        }
        "rm" => {
            exact(a, 2)?;
            let id = s.files.resolve(s.cwd, argument(a, 1)?)?;
            if id == s.cwd {
                return Err(Error::Usage);
            }
            s.files.remove(id)?;
        }
        _ => return Err(Error::Unknown),
    }
    Ok(())
}
