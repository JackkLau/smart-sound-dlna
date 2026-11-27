//! Narrow bindings to the SQLite library shipped with macOS/Linux and Windows 10+.
//! Connections and statements stay on the thread that opens them.
use std::{
    ffi::{c_char, c_int, c_uchar, c_void, CStr, CString},
    path::Path,
    ptr,
};

#[cfg_attr(windows, link(name = "winsqlite3"))]
#[cfg_attr(not(windows), link(name = "sqlite3"))]
unsafe extern "C" {
    fn sqlite3_open(filename: *const c_char, db: *mut *mut c_void) -> c_int;
    fn sqlite3_close(db: *mut c_void) -> c_int;
    fn sqlite3_busy_timeout(db: *mut c_void, milliseconds: c_int) -> c_int;
    fn sqlite3_errmsg(db: *mut c_void) -> *const c_char;
    fn sqlite3_exec(
        db: *mut c_void,
        sql: *const c_char,
        callback: Option<
            unsafe extern "C" fn(*mut c_void, c_int, *mut *mut c_char, *mut *mut c_char) -> c_int,
        >,
        context: *mut c_void,
        error: *mut *mut c_char,
    ) -> c_int;
    fn sqlite3_prepare_v2(
        db: *mut c_void,
        sql: *const c_char,
        len: c_int,
        statement: *mut *mut c_void,
        tail: *mut *const c_char,
    ) -> c_int;
    fn sqlite3_finalize(statement: *mut c_void) -> c_int;
    fn sqlite3_bind_text(
        statement: *mut c_void,
        index: c_int,
        value: *const c_char,
        length: c_int,
        destructor: Option<unsafe extern "C" fn(*mut c_void)>,
    ) -> c_int;
    fn sqlite3_step(statement: *mut c_void) -> c_int;
    fn sqlite3_column_text(statement: *mut c_void, column: c_int) -> *const c_uchar;
    fn sqlite3_column_int(statement: *mut c_void, column: c_int) -> c_int;
    fn sqlite3_changes(db: *mut c_void) -> c_int;
}

pub(crate) struct Connection(*mut c_void);

impl Connection {
    pub fn open(path: &Path) -> Result<Self, String> {
        let path = path.to_str().ok_or("片库路径不是有效的 Unicode 路径")?;
        let name = CString::new(path).map_err(|_| "片库路径无效")?;
        let mut handle = ptr::null_mut();
        let rc = unsafe { sqlite3_open(name.as_ptr(), &mut handle) };
        if handle.is_null() {
            return Err("无法打开 SQLite 片库".into());
        }
        let connection = Self(handle);
        connection.check(rc)?;
        connection.check(unsafe { sqlite3_busy_timeout(handle, 5000) })?;
        connection.batch("PRAGMA foreign_keys=ON;")?;
        Ok(connection)
    }

    fn error(&self) -> String {
        let detail = unsafe { CStr::from_ptr(sqlite3_errmsg(self.0)) }.to_string_lossy();
        format!("SQLite 片库：{detail}")
    }

    fn check(&self, code: c_int) -> Result<(), String> {
        if code == 0 {
            Ok(())
        } else {
            Err(self.error())
        }
    }

    pub fn batch(&self, sql: &str) -> Result<(), String> {
        let sql = CString::new(sql).map_err(|_| "SQL 包含空字符")?;
        self.check(unsafe {
            sqlite3_exec(self.0, sql.as_ptr(), None, ptr::null_mut(), ptr::null_mut())
        })
    }

    pub fn transaction<T>(&self, body: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
        self.batch("BEGIN IMMEDIATE")?;
        let result = body().and_then(|value| {
            self.batch("COMMIT")?;
            Ok(value)
        });
        if result.is_err() {
            let _ = self.batch("ROLLBACK");
        }
        result
    }

    pub fn query(&self, sql: &str, arguments: &[&str]) -> Result<Rows<'_>, String> {
        let sql = CString::new(sql).map_err(|_| "SQL 包含空字符")?;
        let bindings: Vec<_> = arguments
            .iter()
            .map(|s| CString::new(*s).map_err(|_| "字段包含空字符"))
            .collect::<Result<_, _>>()?;
        let mut handle = ptr::null_mut();
        self.check(unsafe {
            sqlite3_prepare_v2(self.0, sql.as_ptr(), -1, &mut handle, ptr::null_mut())
        })?;
        let rows = Rows {
            handle,
            connection: self,
            bindings,
        };
        for (i, arg) in rows.bindings.iter().enumerate() {
            // SQLITE_STATIC is safe: Rows owns the buffers and finalizes the statement before freeing them.
            self.check(unsafe {
                sqlite3_bind_text(handle, (i + 1) as c_int, arg.as_ptr(), -1, None)
            })?;
        }
        Ok(rows)
    }

    pub fn execute(&self, sql: &str, arguments: &[&str]) -> Result<usize, String> {
        if self.query(sql, arguments)?.next()? {
            return Err("写入片库时意外返回了数据行".into());
        }
        Ok(unsafe { sqlite3_changes(self.0) } as usize)
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        unsafe {
            sqlite3_close(self.0);
        }
    }
}

pub(crate) struct Rows<'a> {
    handle: *mut c_void,
    connection: &'a Connection,
    bindings: Vec<CString>,
}

impl Rows<'_> {
    pub fn next(&mut self) -> Result<bool, String> {
        match unsafe { sqlite3_step(self.handle) } {
            100 => Ok(true),  // SQLITE_ROW
            101 => Ok(false), // SQLITE_DONE
            _ => Err(self.connection.error()),
        }
    }

    pub fn text(&self, column: c_int) -> Result<String, String> {
        let value = unsafe { sqlite3_column_text(self.handle, column) };
        if value.is_null() {
            return Err("片库记录缺少文本字段".into());
        }
        unsafe { CStr::from_ptr(value.cast()) }
            .to_str()
            .map(str::to_owned)
            .map_err(|_| "片库记录包含无效文本".into())
    }

    pub fn integer(&self, column: c_int) -> i32 {
        unsafe { sqlite3_column_int(self.handle, column) }
    }
}

impl Drop for Rows<'_> {
    fn drop(&mut self) {
        unsafe {
            sqlite3_finalize(self.handle);
        }
    }
}
