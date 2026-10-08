use http::Method;

const ROUTES: &str = include_str!("../python_routes.txt");

pub fn routes() -> impl Iterator<Item = (Method, &'static str, &'static str)> {
    ROUTES.lines().filter_map(parse_line)
}

pub fn parse_line(line: &str) -> Option<(Method, &str, &str)> {
    let mut fields = line.split(' ');
    let method = parse_method(fields.next()?)?;
    let path = fields.next()?;
    let file = fields.next()?;
    let source_path = file.strip_prefix("src/litellm_lens/")?;

    if fields.next().is_some()
        || !path.starts_with('/')
        || path.chars().any(char::is_whitespace)
        || !file.ends_with(".py")
        || source_path
            .split('/')
            .any(|component| matches!(component, "" | "." | ".."))
    {
        return None;
    }

    Some((method, path, file))
}

fn parse_method(method: &str) -> Option<Method> {
    match method {
        "CONNECT" => Some(Method::CONNECT),
        "DELETE" => Some(Method::DELETE),
        "GET" => Some(Method::GET),
        "HEAD" => Some(Method::HEAD),
        "OPTIONS" => Some(Method::OPTIONS),
        "PATCH" => Some(Method::PATCH),
        "POST" => Some(Method::POST),
        "PUT" => Some(Method::PUT),
        "TRACE" => Some(Method::TRACE),
        _ => None,
    }
}
