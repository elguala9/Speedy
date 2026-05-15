require "net/http"
require "uri"
require "json"

# Minimal HTTP client with retry logic and JSON helpers.
class HttpClient
  DEFAULT_HEADERS = { "User-Agent" => "SpeedyBot/1.0", "Accept" => "application/json" }.freeze

  def initialize(base_url, timeout: 10, retries: 3)
    @base_uri = URI.parse(base_url)
    @timeout  = timeout
    @retries  = retries
  end

  def get(path, params: {})
    uri       = build_uri(path, params)
    request   = Net::HTTP::Get.new(uri, DEFAULT_HEADERS)
    execute(uri, request)
  end

  def post(path, body)
    uri     = build_uri(path)
    request = Net::HTTP::Post.new(uri, DEFAULT_HEADERS.merge("Content-Type" => "application/json"))
    request.body = body.to_json
    execute(uri, request)
  end

  private

  def build_uri(path, params = {})
    uri       = URI.join(@base_uri.to_s + "/", path.delete_prefix("/"))
    uri.query = URI.encode_www_form(params) unless params.empty?
    uri
  end

  def execute(uri, request, attempt: 1)
    response = Net::HTTP.start(uri.host, uri.port,
                               use_ssl: uri.scheme == "https",
                               read_timeout: @timeout) { |http| http.request(request) }
    parse_response(response)
  rescue Net::OpenTimeout, Net::ReadTimeout, Errno::ECONNREFUSED => e
    raise if attempt >= @retries

    sleep(0.2 * 2**attempt)
    execute(uri, request, attempt: attempt + 1)
  end

  def parse_response(response)
    body = response.body
    return JSON.parse(body) if response["Content-Type"]&.include?("application/json")

    body
  end
end
