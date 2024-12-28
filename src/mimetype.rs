use crate::*;

pub fn lookup_mimetype(path: &PathBuf) -> &'static str {
    match path.extension().unwrap_or_default().to_str() {
        Some("doc") => "application/msword",
        Some("md") => "text/markdown",
        Some("pdf") => "application/pdf",
        Some("docx") => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        Some("gif") => "image/gif",
        Some("png") => "image/png",
        Some("jpeg") => "image/jpeg",
        Some("jpg") => "image/jpeg",
        Some("ico") => "image/x-icon",
        Some("htm") => "text/html",
        Some("html") => "text/html",
        Some("txt") => "text/plain",
        Some("jar") => "application/java-archive",
        Some("js") => "text/javascript",
        Some("json") => "application/json",
        Some("zip") => "application/zip",
        Some("gz") => "application/gzip",
        Some("xml") => "application/xml",
        Some("xls") => "application/vnd.ms-excel",
        Some("xlsx") => "application/vnd.ms-excel",
        Some("rtf") => "application/rtf",
        _ => "application/octet-stream",
    }
}
