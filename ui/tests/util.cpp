import waywallen;

namespace
{

struct Case {
    const char* input;
    const char* expected;
};

} // namespace

int main() {
    const Case cases[] = {
        { "[url]http://example.com[/url]",
          "<a href=\"http://example.com\">http://example.com</a>" },
        { "[url=http://example.com]text[/url]",
          "<a href=\"http://example.com\">text</a>" },
        { "[url=http://target.example]http://label.example[/url]",
          "<a href=\"http://target.example\">http://label.example</a>" },
        { "See http://example.com now",
          "See <a href=\"http://example.com\">http://example.com</a> now" },
        { "[url=http://one.example]one[/url] and http://two.example",
          "<a href=\"http://one.example\">one</a> and "
          "<a href=\"http://two.example\">http://two.example</a>" },
        { "[img]http://example.com/image.png[/img]",
          "<img src=\"http://example.com/image.png\">" },
    };

    waywallen::Util util(nullptr);
    for (const auto& test : cases) {
        if (util.bbcodeToHtml(QString::fromUtf8(test.input)) != QString::fromUtf8(test.expected)) {
            return 1;
        }
    }
    return 0;
}
