//! 直传 OBS：用平台签发的临时凭证把文件 PUT 上去。
//!
//! ## ⚠️ 这里有两套签名，别混
//!
//! | | 用在哪 | 算法 | 密钥处理 |
//! |---|---|---|---|
//! | **本模块** | OBS 数据面（PUT 对象） | `AWS4-HMAC-SHA256` | 四级派生 |
//! | 服务端 `iam_sign.go` | 华为 IAM（换临时凭证） | `SDK-HMAC-SHA256` | 单次 HMAC |
//!
//! 两者的算法名、时间头、StringToSign 段数、密钥派生全都不一样。服务端那份
//! 是给 IAM 用的，**与这里无关** —— 不要因为都叫「华为」就互相对照。
//!
//! ## 临时凭证相对长期凭证**唯一的**签名差异
//!
//! `x-amz-security-token` 必须**进 signedHeaders**。不带它、或带了但没签，
//! OBS 一律 403，而错误信息只说「签名不匹配」，不会告诉你是漏了这个头。
//! 这是本模块最容易漏、也最难排查的一条。
//!
//! ## 不新增依赖
//!
//! `ring` 已在依赖里（`digest::SHA256` + `hmac`），够用，不需要引 S3 crate。

use ring::{digest, hmac};

/// 平台签发的临时凭证与落点信息（对应服务端 `storage.UploadTicket`）。
pub struct Ticket {
    pub endpoint: String,
    pub region: String,
    pub bucket: String,
    pub path_style: bool,
    pub access_key: String,
    pub secret_key: String,
    pub security_token: String,
}

/// 服务端为单个文件定下的落点（对应 `storage.TicketObject`）。
///
/// **客户端只负责 PUT 到 `key`、把 `url` 原样回传**，永远不自己拼 URL ——
/// 那样能消掉一整类「两边拼法不一致」的 bug。
pub struct Object {
    pub name: String,
    pub key: String,
    pub url: String,
    pub content_type: String,
}

/// 把字节 PUT 到对象键，返回 ETag。
///
/// `bytes` 整块读进内存：单文件上限 50MiB（服务端常量），这个量级可以接受；
/// 换成流式反而要处理 ureq 的 chunked 编码，而 OBS 对「已签名的固定载荷」
/// 不接受 chunked（除非改用 `STREAMING-AWS4-HMAC-SHA256-PAYLOAD` 那套，成本陡增）。
pub fn put_object(t: &Ticket, key: &str, bytes: &[u8], content_type: &str) -> Result<String, String> {
    let host = host_of(&t.endpoint)?;
    // 虚拟主机式 = 桶名作子域（OBS 控制台「访问域名」就是这个形态）；
    // 路径式则把桶名放进路径。**以服务端下发的 path_style 为准**。
    let (vhost, path) = if t.path_style {
        (host.clone(), format!("/{}/{}", t.bucket, key))
    } else {
        (format!("{}.{}", t.bucket, host), format!("/{}", key))
    };
    let scheme = if t.endpoint.starts_with("http://") { "http" } else { "https" };
    let url = format!("{scheme}://{vhost}{path}");

    let payload_hash = hex(digest::digest(&digest::SHA256, bytes).as_ref());
    let now = time::now_utc();
    let amz_date = now.amz_date();
    let date_stamp = now.date_stamp();

    // canonicalHeaders **必须按头名升序**，signedHeaders 的顺序与之一致。
    // 这个顺序（content-type < host < x-amz-*）是字典序，不是随手排的。
    const SIGNED: &str = "content-type;host;x-amz-content-sha256;x-amz-date;x-amz-security-token";
    let canonical_headers = format!(
        "content-type:{ct}\nhost:{vhost}\nx-amz-content-sha256:{ph}\nx-amz-date:{ad}\nx-amz-security-token:{st}\n",
        ct = content_type.trim(),
        ph = payload_hash,
        ad = amz_date,
        st = t.security_token.trim(),
    );

    // 键只含 `[a-z0-9._/-]`（服务端 sanitize 过），所以 canonical URI
    // **逐字节等于**实际路径 —— SigV4 的 URI 编码坑在这里无从发生。
    let canonical_request = format!(
        "PUT\n{path}\n\n{canonical_headers}\n{SIGNED}\n{payload_hash}"
    );
    let scope = format!("{date_stamp}/{region}/s3/aws4_request", region = t.region);
    let string_to_sign = format!(
        "AWS4-HMAC-SHA256\n{amz_date}\n{scope}\n{}",
        hex(digest::digest(&digest::SHA256, canonical_request.as_bytes()).as_ref())
    );

    let signing_key = signing_key(&t.secret_key, &date_stamp, &t.region);
    let signature = hex(&hmac_sha256(&signing_key, string_to_sign.as_bytes()));

    let authorization = format!(
        "AWS4-HMAC-SHA256 Credential={ak}/{scope}, SignedHeaders={SIGNED}, Signature={signature}",
        ak = t.access_key,
    );

    // 独立自建 agent：OBS 与平台是**两个不同的 host**，不该带上平台的认证头
    // （`Client::decorate` 加的是 X-API-Key-* / Bearer，发给 OBS 只会碍事）。
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(15))
        .timeout_read(std::time::Duration::from_secs(300))
        .user_agent(concat!("ratsa-harness/", env!("CARGO_PKG_VERSION")))
        .build();

    // `send_bytes` 会带 Content-Length。**不要用 `send(reader)`** ——
    // ureq 对未知长度的 Read 会发 `Transfer-Encoding: chunked`，而 OBS 对
    // 已签名的固定载荷不接受 chunked。
    let resp = agent
        .put(&url)
        .set("Content-Type", content_type)
        .set("x-amz-date", &amz_date)
        .set("x-amz-content-sha256", &payload_hash)
        .set("x-amz-security-token", &t.security_token)
        .set("Authorization", &authorization)
        .send_bytes(bytes);

    match resp {
        Ok(r) => Ok(r.header("etag").unwrap_or("").trim_matches('"').to_string()),
        Err(ureq::Error::Status(code, r)) => {
            let body = r.into_string().unwrap_or_default();
            Err(format!(
                "HTTP {code}：{}",
                body.trim().chars().take(300).collect::<String>()
            ))
        }
        Err(ureq::Error::Transport(e)) => Err(format!("网络错误：{e}")),
    }
}

/// 从 endpoint 里取 host（去 scheme、去尾斜杠）。
///
/// ⚠️ **必须先剥 scheme、再去尾斜杠**。反过来写的话，`"https://"` 会先被削成
/// `"https:/"`，scheme 就匹配不上了 —— 本该报错却返回了一个残缺的 host。
fn host_of(endpoint: &str) -> Result<String, String> {
    let h = endpoint
        .trim()
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_end_matches('/');
    if h.is_empty() {
        return Err(format!("票据里的 endpoint 不合法：{endpoint}"));
    }
    Ok(h.to_string())
}

/// SigV4 的四级密钥派生：`AWS4`+SK → 日期 → 区域 → 服务（`s3`）→ `aws4_request`。
///
/// **与华为 IAM 的「单次 HMAC」完全不同**（见服务端 `iam_sign.go` 的对照表）。
fn signing_key(secret: &str, date_stamp: &str, region: &str) -> Vec<u8> {
    let k = hmac_sha256(format!("AWS4{secret}").as_bytes(), date_stamp.as_bytes());
    let k = hmac_sha256(&k, region.as_bytes());
    let k = hmac_sha256(&k, b"s3");
    hmac_sha256(&k, b"aws4_request")
}

fn hmac_sha256(key: &[u8], data: &[u8]) -> Vec<u8> {
    let k = hmac::Key::new(hmac::HMAC_SHA256, key);
    hmac::sign(&k, data).as_ref().to_vec()
}

fn hex(b: &[u8]) -> String {
    use std::fmt::Write;
    b.iter().fold(String::with_capacity(b.len() * 2), |mut s, x| {
        let _ = write!(s, "{x:02x}");
        s
    })
}

// ————————————————————————————————————————————————————————————— UTC 时间
//
// 只为两个日期格式手写一份，不值得为此引入 chrono/time crate。
mod time {
    /// 当前 UTC 时间，做成「自 1970 年起的秒数」+ 两个格式化方法。
    pub struct Utc {
        secs: i64,
    }

    pub fn now_utc() -> Utc {
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        Utc { secs }
    }

    /// 仅供测试构造一个确定的时间点 —— 公历换算容易写错，要用已知日期钉住。
    #[cfg(test)]
    pub fn at(secs: i64) -> Utc {
        Utc { secs }
    }

    impl Utc {
        /// `YYYYMMDDTHHMMSSZ` —— `x-amz-date` 的值。
        pub fn amz_date(&self) -> String {
            let (y, m, d, hh, mm, ss) = self.parts();
            format!("{y:04}{m:02}{d:02}T{hh:02}{mm:02}{ss:02}Z")
        }
        /// `YYYYMMDD` —— 密钥派生与 Credential Scope 用。
        pub fn date_stamp(&self) -> String {
            let (y, m, d, _, _, _) = self.parts();
            format!("{y:04}{m:02}{d:02}")
        }
        /// 公历换算（Howard Hinnant 的 days_from_civil 逆运算）。
        fn parts(&self) -> (i64, u32, u32, u32, u32, u32) {
            let days = self.secs.div_euclid(86_400);
            let rem = self.secs.rem_euclid(86_400);
            let (hh, mm, ss) = (rem / 3600, (rem % 3600) / 60, rem % 60);

            let z = days + 719_468;
            let era = z.div_euclid(146_097);
            let doe = z.rem_euclid(146_097);
            let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
            let y = yoe + era * 400;
            let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
            let mp = (5 * doy + 2) / 153;
            let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
            let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
            let y = if m <= 2 { y + 1 } else { y };
            (y, m, d, hh as u32, mm as u32, ss as u32)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utc_formats_match_sigv4_expectations() {
        // 2021-01-01T00:00:00Z = 1609459200
        let t = time::at(1_609_459_200);
        assert_eq!(t.date_stamp(), "20210101");
        assert_eq!(t.amz_date(), "20210101T000000Z");
        // 跨闰年边界：2020-02-29T12:34:56Z = 1582979696
        let t = time::at(1_582_979_696);
        assert_eq!(t.date_stamp(), "20200229");
        assert_eq!(t.amz_date(), "20200229T123456Z");
    }

    #[test]
    fn host_strips_scheme_and_trailing_slash() {
        assert_eq!(
            host_of("https://obs.ap-southeast-3.myhuaweicloud.com/").unwrap(),
            "obs.ap-southeast-3.myhuaweicloud.com"
        );
        assert!(host_of("https://").is_err());
    }

    #[test]
    fn signing_key_is_derived_not_raw() {
        // 四级派生 ⇒ 结果既不等于 SK，也不等于单次 HMAC(SK, ...)。
        // 这条是对「别把服务和 IAM 那套混用」的机械提醒。
        let k = signing_key("SECRET", "20210101", "ap-southeast-3");
        assert_ne!(k.as_slice(), b"SECRET");
        assert_ne!(k, hmac_sha256(b"SECRET", b"20210101"));
        assert_eq!(k.len(), 32);
    }
}
