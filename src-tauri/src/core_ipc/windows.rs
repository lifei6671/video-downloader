use super::profile_key;
use std::{
    ffi::c_void,
    io,
    mem::size_of,
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    path::Path,
    ptr,
};
use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};
use windows_sys::Win32::{
    Foundation::{LocalFree, ERROR_INSUFFICIENT_BUFFER},
    Security::{
        Authorization::{
            ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
            SDDL_REVISION_1,
        },
        GetTokenInformation, TokenUser, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER,
    },
    System::Threading::{GetCurrentProcess, OpenProcessToken},
};

struct LocalAllocation(*mut c_void);
impl Drop for LocalAllocation {
    fn drop(&mut self) {
        // SAFETY: These non-null buffers were allocated by the Win32 conversion APIs.
        unsafe {
            LocalFree(self.0);
        }
    }
}

pub(super) fn current_user_sid() -> io::Result<String> {
    let mut token = ptr::null_mut();
    // SAFETY: Current process handle is valid; token is a writable output pointer.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: Successful OpenProcessToken transfers ownership of this handle.
    let token = unsafe { OwnedHandle::from_raw_handle(token) };
    let mut length = 0;
    // SAFETY: Null buffer with size zero queries the required TOKEN_USER buffer size.
    let queried = unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            TokenUser,
            ptr::null_mut(),
            0,
            &mut length,
        )
    };
    if queried != 0
        || io::Error::last_os_error().raw_os_error() != Some(ERROR_INSUFFICIENT_BUFFER as i32)
    {
        return Err(io::Error::last_os_error());
    }
    // usize storage provides TOKEN_USER alignment and space for its trailing SID.
    let mut buffer = vec![0usize; (length as usize).div_ceil(size_of::<usize>())];
    // SAFETY: Buffer is aligned, writable and at least length bytes, and token stays live.
    if unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            TokenUser,
            buffer.as_mut_ptr().cast(),
            length,
            &mut length,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: Successful query initializes TOKEN_USER and the SID within the live buffer.
    let sid = unsafe { (*(buffer.as_ptr().cast::<TOKEN_USER>())).User.Sid };
    let mut text = ptr::null_mut();
    // SAFETY: sid points into the live buffer; text is a writable output pointer.
    if unsafe { ConvertSidToStringSidW(sid, &mut text) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let allocation = LocalAllocation(text.cast());
    let mut size = 0;
    // SAFETY: Conversion returns a valid NUL-terminated UTF-16 string until LocalFree.
    unsafe {
        while *text.add(size) != 0 {
            size += 1;
        }
        let result = String::from_utf16_lossy(std::slice::from_raw_parts(text, size));
        drop(allocation);
        Ok(result)
    }
}

fn create_pipe(endpoint: &str, sid: &str, first: bool) -> io::Result<NamedPipeServer> {
    // No inherited/default ACEs: only the current user receives pipe access.
    let sddl: Vec<u16> = format!("D:P(A;;GA;;;{sid})\0").encode_utf16().collect();
    let mut descriptor = ptr::null_mut();
    // SAFETY: Input is terminated UTF-16; descriptor is a valid output pointer.
    if unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            SDDL_REVISION_1,
            &mut descriptor,
            ptr::null_mut(),
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    let descriptor = LocalAllocation(descriptor);
    let mut attributes = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor.0,
        bInheritHandle: 0,
    };
    let mut options = ServerOptions::new();
    options
        .first_pipe_instance(first)
        .reject_remote_clients(true);
    // SAFETY: Both attributes and its descriptor stay valid throughout CreateNamedPipe.
    // Windows copies the security descriptor; no pointers outlive this call.
    unsafe {
        options.create_with_security_attributes_raw(
            endpoint,
            (&mut attributes as *mut SECURITY_ATTRIBUTES).cast(),
        )
    }
}

pub(super) struct Listener {
    endpoint: String,
    sid: String,
    pending: NamedPipeServer,
}
impl Listener {
    pub(super) async fn bind(profile: &Path) -> io::Result<Self> {
        let sid = current_user_sid()?;
        let endpoint = format!(r"\\.\pipe\yingqu-mcp-v1-{}", profile_key(profile, &sid));
        let pending = create_pipe(&endpoint, &sid, true)?;
        Ok(Self {
            endpoint,
            sid,
            pending,
        })
    }
    pub(super) fn endpoint(&self) -> &str {
        &self.endpoint
    }
    pub(super) async fn accept(&mut self) -> io::Result<NamedPipeServer> {
        self.pending.connect().await?;
        // Keep a listening instance alive while serving existing clients.
        let next = create_pipe(&self.endpoint, &self.sid, false)?;
        Ok(std::mem::replace(&mut self.pending, next))
    }
}

#[cfg(test)]
fn pipe_dacl(stream: &impl AsRawHandle) -> io::Result<String> {
    use windows_sys::Win32::Security::{
        Authorization::{
            ConvertSecurityDescriptorToStringSecurityDescriptorW, GetSecurityInfo, SE_KERNEL_OBJECT,
        },
        DACL_SECURITY_INFORMATION,
    };
    let mut descriptor = ptr::null_mut();
    // SAFETY: Pipe handle is live and descriptor is a writable output pointer.
    let code = unsafe {
        GetSecurityInfo(
            stream.as_raw_handle(),
            SE_KERNEL_OBJECT,
            DACL_SECURITY_INFORMATION,
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            &mut descriptor,
        )
    };
    if code != 0 {
        return Err(io::Error::from_raw_os_error(code as i32));
    }
    let allocation = LocalAllocation(descriptor);
    let mut text = ptr::null_mut();
    let mut length = 0;
    // SAFETY: descriptor is valid; the API allocates the output string.
    if unsafe {
        ConvertSecurityDescriptorToStringSecurityDescriptorW(
            allocation.0,
            SDDL_REVISION_1,
            DACL_SECURITY_INFORMATION,
            &mut text,
            &mut length,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    let text_allocation = LocalAllocation(text.cast());
    // SAFETY: API returns length UTF-16 code units including the terminator.
    let result =
        unsafe { String::from_utf16_lossy(std::slice::from_raw_parts(text, length as usize - 1)) };
    drop(text_allocation);
    Ok(result)
}

#[cfg(test)]
#[tokio::test]
async fn pipe_has_a_protected_dacl_for_only_the_current_user() {
    let root = tempfile::tempdir().unwrap();
    let listener = Listener::bind(&root.path().join("fixture.sqlite"))
        .await
        .unwrap();
    let descriptor = pipe_dacl(&listener.pending).unwrap();
    assert!(
        descriptor.starts_with("D:P"),
        "DACL was not protected: {descriptor}"
    );
    assert_eq!(
        descriptor.matches('(').count(),
        1,
        "unexpected ACEs: {descriptor}"
    );
    assert!(
        descriptor.contains(&current_user_sid().unwrap()),
        "wrong user: {descriptor}"
    );
    assert!(
        descriptor.contains("(A;;"),
        "missing allow ACE: {descriptor}"
    );
}
