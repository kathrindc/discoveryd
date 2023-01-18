use std::{sync::atomic::{AtomicUsize, Ordering}, time::SystemTime};

use regex::Regex;
use rocket::{http::{ContentType, Status, Method}, request::{FromRequest, Outcome}, Request, outcome::IntoOutcome, fairing::{Fairing, Info, Kind, self}, futures::task::Spawn, Response, Rocket, Build, State, config::Config, figment::{Figment, providers::{Serialized, Env, Toml, Format}, Profile}};
use rocket_db_pools::{Database, sqlx::{self, Row}, Connection};
use lazy_static::lazy_static;

#[macro_use] extern crate rocket;

#[derive(Database)]
#[database("discoveryd")]
struct AppDb(sqlx::MySqlPool);

struct HostInfo {
    pub domain: String,
    pub sts_mode: String,
    pub sts_mx: Vec<String>,
    pub imap_server: String,
    pub imap_port: i32,
    pub imap_ssl: bool,
    pub smtp_server: String,
    pub smtp_port: i32,
    pub smtp_ssl: bool,
    pub activesync_url: Option<String>,
    pub activesync_preferred: bool
}

struct ServiceStat {
    mtasts: AtomicUsize,
    autodiscover: AtomicUsize,
    autoconfig: AtomicUsize,
    started_at: SystemTime
}

#[rocket::async_trait]
impl <'r> FromRequest<'r> for HostInfo {
    type Error = ();

    async fn from_request(req: &'r Request<'_>) -> Outcome<Self, ()> {
        let domain_result = req
            .host()
            .and_then(|host| Some(host.domain().to_string()));

        if !domain_result.is_some() {
            return Outcome::Failure((Status::BadRequest, ()));
        }

        let domain = domain_result.unwrap();
        let connection_result = req
            .guard::<Connection<AppDb>>()
            .await;

        if !connection_result.is_success() {
            return Outcome::Failure((Status::ServiceUnavailable, ()));
        }

        let mut connection = connection_result.unwrap();
        let row_result = sqlx
            ::query("SELECT id, domain, sts_mode, imap_server, imap_port, imap_ssl, smtp_server, smtp_port, smtp_ssl, activesync_url, activesync_preferred FROM domains WHERE domain = ?")
            .bind(domain)
            .fetch_optional(&mut *connection)
            .await;

        if !row_result.is_ok() {
            return Outcome::Failure((Status::ServiceUnavailable, ()));
        }

        let row_option = row_result.unwrap();

        if !row_option.is_some() {
            return Outcome::Forward(());
        }

        let row = row_option.unwrap();
        let domain_id: i32 = row.get("id");
        let mxs_result = sqlx
            ::query("SELECT host FROM mx_whitelists WHERE domain_id = ?")
            .bind(domain_id)
            .fetch_all(&mut *connection)
            .await;

        if !mxs_result.is_ok() {
            return Outcome::Failure((Status::ServiceUnavailable, ()));
        }

        let mxs = mxs_result
            .unwrap()
            .into_iter()
            .map(|row| row.get("host"))
            .collect();

        Outcome::Success(HostInfo {
            domain: row.get("domain"),
            sts_mode: row.get("sts_mode"),
            sts_mx: mxs,
            imap_server: row.get("imap_server"),
            imap_port: row.get("imap_port"),
            imap_ssl: row.get("imap_ssl"),
            smtp_server: row.get("smtp_server"),
            smtp_port: row.get("smtp_port"),
            smtp_ssl: row.get("smtp_ssl"),
            activesync_url: row.get("activesync_url"),
            activesync_preferred: row.get("activesync_preferred")
        })
    }
}

#[get("/")]
fn index(stat: &State<ServiceStat>) -> (Status, (ContentType, String)) {
    let content = format!(
        "<!DOCTYPE html><html><head><meta charset=\"utf-8\"><title>freediscover.toast.ws</title><style>* {{ background: #292929; color: white; font-family: sans-serif; font-weight: normal; }} html, body {{ margin: 0; padding: 0; width: 100vw; height: 100vh; }} body {{ display: flex; flex-direction: column; justify-content: center; align-items: center; }}</style></head><body><h1>discoveryd @ <a href=\"https://freediscover.toast.ws\">freediscover.toast.ws</a></h1><p><b>MTA-STS:</b> {}<br><b>AutoDiscover:</b> {}<br><b>AutoConfig:</b> {}<br><b>Uptime:</b> {} seconds</p></body></html>",
        stat.mtasts.load(Ordering::Relaxed),
        stat.autodiscover.load(Ordering::Relaxed),
        stat.autoconfig.load(Ordering::Relaxed),
        stat.started_at.elapsed().unwrap_or_default().as_secs(),
    );

    (Status::Ok, (ContentType::HTML, content))
}

#[get("/.well-known/mta-sts.txt")]
fn mta_sts(info: HostInfo, stat: &State<ServiceStat>) -> (Status, (ContentType, String)) {
    let mxs = info.sts_mx
        .into_iter()
        .map(|mx| format!("mx: {mx}\n"))
        .collect::<Vec<String>>()
        .join("");
    let content = format!("version: STSv1\nmode: {}\n{}max-age: 86400\n", info.sts_mode, mxs);

    stat.mtasts.fetch_add(1, Ordering::Relaxed);

    (Status::Ok, (ContentType::Text, content))
}

#[post("/autodiscover/autodiscover.xml", data = "<data>")]
fn autodiscover(info: HostInfo, data: String, stat: &State<ServiceStat>) -> (Status, (ContentType, String)) {
    lazy_static! {
        static ref RE: Regex = Regex::new(r"<EMailAddress>(.*?)</EMailAddress>").unwrap();
    }

    let address_option = RE.captures(data.as_str())
        .and_then(|cap| 
            cap.get(1).map(|value| value.as_str().to_string())
        );

    stat.autodiscover.fetch_add(1, Ordering::Relaxed);

    match address_option {
        Some(address) => {
            let result = match info.activesync_preferred {
                true => format!(
r#"
<Autodiscover xmlns="http://schemas.microsoft.com/exchange/autodiscover/responseschema/2006">"
    <Response xmlns="http://schemas.microsoft.com/exchange/autodiscover/outlook/responseschema/2006a">
        <Culture>en:en</Culture>
        <User>
            <DisplayName>{}</DisplayName>
            <EMailAddress>{}</EMailAddress>
        </User>
        <Action>
            <Settings>
                <Server>
                    <Type>MobileSync</Type>
                    <Url>{}</Url>
                    <Name>{}</Name>
                </Server>
            </Settings>
        </Action>
    </Response>
</Autodiscover>
"#,
                    address,
                    address,
                    info.activesync_url.clone().unwrap_or_default(),
                    info.activesync_url.clone().unwrap_or_default()
                ),

                false => format!(
r#"<?xml version="1.0" encoding="utf-8" ?>
<Autodiscover xmlns="http://schemas.microsoft.com/exchange/autodiscover/responseschema/2006">"
    <Response xmlns="http://schemas.microsoft.com/exchange/autodiscover/outlook/responseschema/2006a">
        <Account>
            <AccountType>email</AccountType>
            <Action>settings</Action>
            <Protocol>
                <Type>IMAP</Type>
                <Server>{}</Server>
                <Port>{}</Port>
                <DomainRequired>off</DomainRequired>
                <LoginName>{}</LoginName>
                <SPA>off</SPA>
                <SSL>{}</SSL>
                <AuthRequired>on</AuthRequired>
            </Protocol>
            <Protocol>
                <Type>SMTP</Type>
                <Server>{}</Server>
                <Port>{}</Port>
                <DomainRequired>off</DomainRequired>
                <LoginName>{}</LoginName>
                <SPA>off</SPA>
                <SSL>{}</SSL>
                <AuthRequired>on</AuthRequired>
                <UsePOPAuth>on</UsePOPAuth>
                <SMTPLast>off</SMTPLast>
            </Protocol>
        </Account>
    </Response>
</Autodiscover>
"#,
                    info.imap_server,
                    info.imap_port,
                    address,
                    match info.imap_ssl {
                        true => "on",
                        false => "off"
                    },
                    info.smtp_server,
                    info.smtp_port,
                    address,
                    match info.smtp_ssl {
                        true => "on",
                        false => "off"
                    }
                ),
            };

            (Status::Ok, (ContentType::XML, result))
        },

        None => {
            let result = format!(
r#"<?xml version="1.0" encoding="utf-8" ?>
<Autodiscover xmlns="http://schemas.microsoft.com/exchange/autodiscover/responseschema/2006">
    <Response>
        <Error Time="{}" Id="2477272013">
            <ErrorCode>600</ErrorCode>
            <Message>Invalid Request</Message>
            <DebugData />
        <Error />
    </Response>
</Autodiscover>"#,
                SystemTime::UNIX_EPOCH.elapsed().unwrap().as_micros()
            );

            (Status::Ok, (ContentType::XML, result))
        }
    }
}

#[get("/mail/config-v1.1.xml?<emailaddress>")]
fn autoconfig(emailaddress: Option<String>, info: HostInfo, stat: &State<ServiceStat>) -> (Status, (ContentType, String)) {
    stat.autoconfig.fetch_add(1, Ordering::Relaxed);

    match emailaddress {
        Some(address) => {
            let result = format!(
r#"<?xml version="1.0" encoding="utf-8" ?>
<clientConfig version="1.1">
    <emailProvider id="{}">
        <domain>{}</domain>
        <displayName>{}</displayName>
        <displayShortName>{}</displayShortName>
        <incomingServer type="imap">
            <hostname>{}</hostname>
            <port>{}</port>
            <socketType>{}</socketType>
            <authentication>password-cleartext</authentication>
            <username>{}</username>
        </incomingServer>
        <outgoingServer type="smtp">
            <hostname>{}</hostname>
            <port>{}</port>
            <socketType>{}</socketType>
            <authentication>password-cleartext</authentication>
            <username>{}</username>
        </outgoingServer>
    </emailProvider>
</clientConfig>"#,
                info.domain,
                info.domain,
                address,
                info.domain,
                info.imap_server,
                info.imap_port,
                match info.imap_ssl {
                    true => "SSL",
                    false => "STARTTLS",
                },
                address,
                info.smtp_server,
                info.smtp_port,
                match info.smtp_ssl {
                    true => "SSL",
                    false => "STARTTLS",
                },
                address
            );

            (Status::Ok, (ContentType::XML, result))
        }

        None => {
            let result = r#"<?xml version="1.0" encoding="utf-8" ?><error>malformed request</error>"#.to_string();

            (Status::BadRequest, (ContentType::XML, result))
        }
    }
}

#[launch]
fn rocket() -> _ {
    let stat = ServiceStat {
        mtasts: AtomicUsize::new(0),
        autodiscover: AtomicUsize::new(0),
        autoconfig: AtomicUsize::new(0),
        started_at: SystemTime::now()
    };
    let figment = Figment::from(rocket::Config::figment())
        .merge(Toml::file("/etc/discoveryd.toml").nested());

    rocket::custom(figment)
        .manage(stat)
        .attach(AppDb::init())
        .mount("/", routes![index, mta_sts, autodiscover, autoconfig])
}
