import { HelpTopicSource } from '../help-model';

/*
 * Vietnamese Help (docs/I18N.md "Help"): the basics, your sessions and
 * phone & remote. Each topic has the same sections in the same order as its
 * English one in `help-topics.ts`; section ids, `where`, embeds and link
 * paths come from the English, only the link labels from here. `**bold**`
 * names a label as the Vietnamese interface shows it. Labels of the
 * operating system (macOS, iOS, Android, Windows) are given in Vietnamese
 * with the English in brackets, since the phone or computer may show either.
 */
const TOPICS: HelpTopicSource[] = [
  // ───────────────────────────────────────────────────────────── Cơ bản
  {
    id: 'getting-started',
    title: 'Bắt đầu',
    icon: 'sparkle',
    group: 'basics',
    summary: 'Cài đặt ứng dụng, kết nối với Claude Code và cấp các quyền cần thiết.',
    intro:
      'Mother Claude chạy trên máy tính nơi bạn dùng Claude Code. Ứng dụng hiện mọi phiên Claude Code trên máy đó, cho bạn điều khiển chúng, và đưa cùng bảng điều khiển ấy lên điện thoại của bạn.',
    keywords: [
      'cài đặt',
      'tải về',
      'thiết lập',
      'mở lần đầu',
      'chào mừng',
      'làm quen',
      'yêu cầu hệ thống',
    ],
    sections: [
      {
        heading: 'Những gì bạn cần',
        points: [
          'Máy Mac chip Apple silicon, hoặc máy PC Windows (x64). Máy Mac chip Intel chưa được hỗ trợ.',
          '**Claude Code CLI** (`claude`), đã cài đặt và đăng nhập trên cùng máy tính. Mother Claude đọc các phiên của nó và dùng nó để bắt đầu phiên mới.',
          'Tùy chọn: `tmux`, để gõ vào các phiên chạy trong một khung tmux.',
        ],
      },
      {
        heading: 'Cài đặt trên máy Mac',
        steps: [
          'Tải tệp `.dmg` từ trang **Releases** của dự án.',
          'Mở tệp đó và kéo **Mother Claude** vào thư mục Applications.',
          'Mở Mother Claude.',
        ],
        tips: [
          'Ứng dụng cho Mac đã được ký và công chứng (notarized), nên macOS mở nó như mọi ứng dụng khác.',
        ],
      },
      {
        heading: 'Cài đặt trên Windows',
        steps: [
          'Tải tệp `…-setup.exe` từ trang **Releases** của dự án (hoặc tệp `.msi` để cài đặt có quản lý).',
          'Chạy tệp đó.',
          'Nếu SmartScreen cảnh báo, chọn **Thông tin thêm** (More info), rồi **Vẫn chạy** (Run anyway).',
          'Mở Mother Claude.',
        ],
        notes: ['SmartScreen cảnh báo vì bộ cài cho Windows chưa được ký.'],
      },
      {
        heading: 'Cấp quyền Truy cập toàn bộ ổ đĩa trên máy Mac',
        where: 'desktop',
        intro:
          'macOS bảo vệ thư mục nơi Claude Code lưu các phiên. Lần đầu bạn mở ứng dụng, màn hình **Chào mừng đến với Mother Claude** sẽ xin quyền **Truy cập toàn bộ ổ đĩa** (Full Disk Access). Windows không cần bước nào như vậy.',
        steps: [
          'Nhấp **Mở Cài đặt hệ thống**.',
          'Trong Quyền riêng tư & Bảo mật › Truy cập toàn bộ ổ đĩa (Privacy & Security › Full Disk Access), tìm **Mother Claude**. Nếu không thấy trong danh sách, nhấp **+** và thêm nó; **Hiện ứng dụng trong Finder** cho biết ứng dụng nằm ở đâu.',
          'Bật công tắc của nó. macOS có thể yêu cầu bạn thoát rồi mở lại ứng dụng.',
          'Quay lại và nhấp **Kiểm tra lại**.',
          'Nhấp **Tiếp tục vào bảng điều khiển →**.',
        ],
        tips: [
          '**Bỏ qua lúc này** vẫn cho bạn vào, nhưng ứng dụng không đọc được các phiên của bạn và một biểu ngữ sẽ hiện cho đến khi bạn làm xong. Bạn có thể quay lại bất cứ lúc nào trong **Cài đặt › Quyền**.',
          'Quyền Mạng cục bộ (Local Network) là tùy chọn: khi macOS hỏi Mother Claude có được tìm thiết bị trên mạng cục bộ không, hãy nhấp **Cho phép** (Allow) để điện thoại của bạn kết nối được tới nó.',
        ],
        links: [{ label: 'Mở trang Cài đặt', path: '/settings' }],
      },
      {
        heading: 'Làm quen với ứng dụng',
        points: [
          'Trên ứng dụng máy tính, thanh bên có **Phiên**, **Hộp thư**, **Tác tử**, **Phiên mới** và **Nhóm**; trong **Dữ liệu của bạn** là **Tài khoản** và **Sao lưu & đồng bộ**; trong **Kết nối** là **Từ xa**, **Trình duyệt** và **API**; trong **Hệ thống** là **Công suất** và **Cài đặt**; cùng **Trợ giúp** ở dưới cùng.',
          'Trên điện thoại, các thẻ ở dưới cùng là **Phiên**, **Hộp thư**, **Tác tử** và **Thêm**. Mọi mục khác nằm trong **Thêm**.',
          'Chấm ở cuối thanh bên (ở trên cùng trên điện thoại) cho biết trạng thái kết nối. **Trực tiếp** nghĩa là cập nhật đến ngay khi có.',
          'Nút bên cạnh đổi giao diện giữa tối, sáng và theo hệ thống. **Cài đặt › Hiển thị** còn có ngôn ngữ, màu nhấn và nền.',
          'Lần đầu, một ghi chú ngắn **Chào mừng đến với Mother Claude** giải thích cách ứng dụng hoạt động. Nhấn **Đã hiểu** để đóng nó.',
        ],
      },
      {
        heading: 'Bước tiếp theo',
        points: [
          'Bắt đầu một phiên từ trang **Phiên mới**: phiên bắt đầu ở đó được điều khiển hoàn toàn.',
          'Ghép nối điện thoại từ trang **Từ xa** trên ứng dụng máy tính.',
          'Để trả lời các phiên bạn bắt đầu trong terminal hay VS Code, hãy cài đặt điều khiển phiên bên ngoài.',
        ],
        links: [
          { label: 'Mở trang Phiên mới', path: '/new' },
          { label: 'Xem “Dùng trên điện thoại”', path: '/help/phone' },
          { label: 'Xem “Phiên bắt đầu ở nơi khác”', path: '/help/foreign-sessions' },
        ],
      },
    ],
  },
  {
    id: 'good-to-know',
    title: 'Điều nên biết',
    icon: 'book',
    group: 'basics',
    summary: 'Cách Mother Claude hoạt động, và những giới hạn nên biết.',
    intro:
      'Đôi điều về cách Mother Claude nối máy tính của bạn với điện thoại, và những gì nó làm được hay không làm được.',
    keywords: [
      'giới hạn',
      'hạn chế',
      'giữ máy thức',
      'ngủ',
      'máy tính xách tay',
      'từng tin nhắn',
      'một nơi điều khiển',
      'ưu tiên cục bộ',
      'quyền riêng tư',
    ],
    sections: [
      { heading: 'Tóm tắt', embed: 'limitations' },
      {
        heading: 'Phiên do Mother Claude chạy, và các phiên khác',
        intro:
          'Mother Claude chỉ điều khiển hoàn toàn được một phiên khi chính nó đã bắt đầu phiên đó: phiên như vậy **thuộc Mother Claude**. Phiên bạn bắt đầu trong terminal, VS Code, JetBrains hay Claude Desktop là phiên bắt đầu ở nơi khác. Bạn luôn xem được chúng, và có thể trả lời, nhắn tin cho chúng qua plugin Mother Claude hoặc tmux.',
        points: [
          'Phiên thuộc Mother Claude nhận tin nhắn, hình ảnh, quyết định duyệt, lệnh đổi mô hình, mức nỗ lực và chế độ, lệnh ngắt và lệnh gạch chéo; câu trả lời của Claude hiện ra từng chữ.',
          '**Tiếp tục tại đây** biến bất kỳ phiên nào thành phiên thuộc Mother Claude. Phiên vẫn đang chạy ở nơi khác được rẽ nhánh thành một bản sao; phiên đã kết thúc thì tiếp tục ngay tại chỗ.',
          'Huy hiệu của mỗi phiên cho biết bạn điều khiển được nó đến đâu: **Điều khiển đầy đủ**, **Qua hook**, **Qua tmux**, **Thử nghiệm** hoặc **Chỉ xem**. Chạm vào huy hiệu để xem những gì dùng được và vì sao.',
        ],
        links: [
          { label: 'Xem “Tin nhắn và điều khiển”', path: '/help/messaging' },
          { label: 'Xem “Phiên bắt đầu ở nơi khác”', path: '/help/foreign-sessions' },
        ],
      },
      {
        heading: 'Những gì ở lại trên máy tính của bạn',
        points: [
          'Phiên, bản ghi và token của bạn ở lại trên máy tính của bạn. Mother Claude không có dịch vụ đám mây nào.',
          'Mỗi điện thoại đã ghép nối có khóa riêng, và bạn có thể thu hồi từng thiết bị một cách riêng lẻ.',
          'Mother Claude không bao giờ sửa cài đặt Claude Code của bạn. Plugin của nó được cài và gỡ bằng chính các lệnh `claude plugin` của Claude Code.',
          'Nó chỉ ghi vào thư mục của Claude Code khi bạn yêu cầu: thêm tài khoản thư mục, di chuyển, sao chép hay xóa cuộc trò chuyện, sao chép tùy chỉnh, khôi phục hoặc kéo về một bản sao lưu.',
        ],
      },
      {
        heading: 'Thao tác rủi ro vẫn do bạn quyết định',
        intro:
          'Một số thao tác là **nguy hiểm**: chúng cho Claude chạy mà không cần duyệt, hoặc thay đổi cài đặt vượt ra ngoài một yêu cầu.',
        points: [
          'Ví dụ: chuyển một phiên sang **Bỏ qua quyền**, lưu quy tắc “luôn cho phép” vào tệp cài đặt, cho phép một lệnh gọi công cụ bị đánh dấu nguy hiểm, gỡ bỏ hoặc xóa phiên, và gửi phím thô vào terminal.',
          'Ứng dụng máy tính luôn làm được các thao tác này. Điện thoại chỉ làm được sau khi bạn cho phép trên ứng dụng máy tính, trong **Từ xa › Thiết bị đã ghép nối**, riêng cho mạng ở nhà và cho internet.',
        ],
        links: [{ label: 'Xem “Truy cập qua internet và thiết bị”', path: '/help/internet' }],
      },
    ],
  },
  {
    id: 'settings',
    title: 'Cài đặt và cập nhật',
    icon: 'settings',
    group: 'basics',
    summary: 'Thông báo, giao diện, mức dùng gói, kết nối và cập nhật phần mềm.',
    intro:
      'Cài đặt chứa những gì áp dụng cho thiết bị này: thông báo, giao diện và kết nối. Trong ứng dụng máy tính, Cài đặt còn có cập nhật phần mềm và các quyền macOS.',
    keywords: [
      'tùy chọn',
      'giao diện',
      'chế độ tối',
      'chế độ sáng',
      'cập nhật',
      'nâng cấp',
      'phiên bản',
    ],
    sections: [
      {
        heading: 'Trong Cài đặt có gì',
        terms: [
          {
            term: 'Thông báo',
            text: 'Thông báo đẩy cho thiết bị này, các sự kiện sẽ báo cho bạn, và cảnh báo trong ứng dụng.',
          },
          {
            term: 'Hiển thị',
            text: 'Ngôn ngữ, giao diện (**Tối**, **Sáng** hoặc **Theo hệ thống**), màu nhấn và nền.',
          },
          { term: 'Tài khoản Claude', text: 'Các trang **Tài khoản** và **Sao lưu & đồng bộ**.' },
          { term: 'Mức dùng gói', text: 'Tần suất đọc giới hạn của gói Claude của bạn.' },
          {
            term: 'Kết nối',
            text: 'Máy chủ, cách thiết bị này kết nối, thiết bị có được thực hiện thao tác nguy hiểm không, và cập nhật trực tiếp.',
          },
          {
            term: 'Ứng dụng Màn hình chính',
            text: 'Mã dùng một lần cho ứng dụng trên Màn hình chính của iPhone (chỉ thiết bị đã ghép nối).',
          },
          {
            term: 'Cập nhật phần mềm',
            text: 'Phiên bản mới của ứng dụng (chỉ ứng dụng máy tính).',
          },
          {
            term: 'Quyền',
            text: 'Các quyền macOS như Truy cập toàn bộ ổ đĩa (chỉ ứng dụng máy tính).',
          },
          { term: 'Giới thiệu', text: 'Phiên bản, **Trợ giúp**, **Dịch vụ** và **HTTP API**.' },
        ],
        links: [{ label: 'Mở trang Cài đặt', path: '/settings' }],
      },
      {
        heading: 'Ngôn ngữ, giao diện và màu sắc',
        steps: [
          'Mở **Cài đặt**.',
          'Trong **Hiển thị**, chọn **Ngôn ngữ**: **English** hoặc **Tiếng Việt**. Toàn bộ ứng dụng đổi ngay.',
          'Chọn **Giao diện**: **Tối** (mặc định), **Sáng**, hoặc **Theo hệ thống** để theo máy tính hay điện thoại của bạn.',
          'Chọn một **Màu nhấn**, và một **Nền**: **Chuẩn**, **Nửa đêm**, **Đen** (đen tuyền, dịu mắt trên màn hình OLED) hoặc **Ấm**.',
        ],
        tips: [
          'Trên ứng dụng máy tính, nút ở cuối thanh bên cũng đổi được giao diện.',
          'Mỗi thiết bị giữ lựa chọn riêng: ứng dụng máy tính và điện thoại của bạn có thể khác nhau.',
        ],
      },
      {
        heading: 'Làm mới mức dùng gói',
        intro:
          'Trang Phiên hiện giới hạn của gói Claude của bạn (khung 5 giờ hiện tại và các khung theo tuần) cùng thời điểm đặt lại.',
        steps: [
          'Mở **Cài đặt** trong ứng dụng máy tính.',
          'Trong **Mức dùng gói**, mở **Làm mới mức dùng gói**.',
          'Chọn **Mỗi 3 phút**, **Mỗi 5 phút (khuyên dùng)**, **Mỗi 10 phút** hoặc **Chỉ thủ công**.',
        ],
        points: [
          'Ứng dụng đọc theo nhịp đó khi có phiên đang làm việc, còn lại thì 30 phút một lần, và chỉ khi có bảng điều khiển đang mở. **Làm mới** trong bảng mức dùng gói sẽ đọc ngay.',
          'Điện thoại hiện cài đặt này; bạn đổi nó trong ứng dụng máy tính.',
        ],
      },
      {
        heading: 'Cập nhật ứng dụng',
        where: 'desktop',
        intro:
          'Ứng dụng máy tính tìm phiên bản mới mỗi khi khởi động. Khi có bản sẵn sàng, **Có bản cập nhật** hiện ở cuối thanh bên.',
        steps: [
          'Nhấp **Có bản cập nhật**, hoặc mở **Cài đặt › Cập nhật phần mềm**.',
          'Để kiểm tra lại ngay, nhấp **Kiểm tra bản cập nhật**.',
          'Đọc ghi chú phát hành.',
          'Nhấp **Tải về & cài đặt**.',
          'Chờ trong lúc ứng dụng hiện **Đang tải về…**, **Đang cài đặt…** và **Đã cài bản cập nhật — đang khởi động lại…**. Ứng dụng tự khởi động lại.',
        ],
        notes: [
          'Trên Windows, ứng dụng đóng lại trong lúc cài bản cập nhật.',
          'Sau khi khởi động lại, HTTP API tắt cho đến khi bạn bật lại nó trên màn hình **API**.',
        ],
      },
      {
        heading: 'Kết nối',
        points: [
          '**Kết nối** cho biết máy chủ mà thiết bị này dùng, cách nó kết nối, nó có được thực hiện thao tác nguy hiểm không, và trạng thái cập nhật trực tiếp.',
          'Nhấn **Kết nối lại** để kết nối lại ngay.',
          'Trong ứng dụng máy tính, **Ghép nối điện thoại** mở phần ghép nối. Trên điện thoại, **Hủy ghép nối thiết bị này** gỡ điện thoại đó.',
        ],
        links: [{ label: 'Xem “Dùng trên điện thoại”', path: '/help/phone' }],
      },
    ],
  },

  // ─────────────────────────────────────────────────── Phiên của bạn
  {
    id: 'sessions',
    title: 'Danh sách phiên',
    icon: 'sessions',
    group: 'sessions',
    summary:
      'Mọi phiên Claude Code trên máy tính của bạn, phiên đó đang làm gì, và cách tìm một phiên.',
    intro:
      'Trang **Phiên** liệt kê mọi phiên Claude Code trên máy tính của bạn đang chạy hoặc có hoạt động trong 24 giờ qua, bất kể thứ gì đã bắt đầu nó: terminal, VS Code, JetBrains, Claude Desktop, một tác vụ nền hay chính Mother Claude.',
    keywords: ['danh sách', 'bộ lọc', 'trạng thái', 'môi trường', 'dự án', 'tác tử', 'tìm phiên'],
    sections: [
      {
        heading: 'Danh sách được nhóm thế nào',
        terms: [
          {
            term: 'Cần bạn',
            text: 'Một yêu cầu cấp quyền, câu hỏi hoặc kế hoạch đang chờ bạn. Mục cũ nhất đứng đầu.',
          },
          { term: 'Đang làm việc', text: 'Claude đang xử lý một lượt, hoặc phiên đang khởi động.' },
          {
            term: 'Sẽ tiếp tục',
            text: 'Lượt đã kết thúc, nhưng việc mà lượt đó bắt đầu vẫn đang chạy, hoặc một lần đánh thức đã hẹn sẽ đánh thức phiên. Phiên chưa xong.',
          },
          { term: 'Rảnh', text: 'Đang chờ tin nhắn tiếp theo của bạn.' },
          { term: 'Vừa kết thúc', text: 'Đã kết thúc, thất bại hoặc bị dừng trong 24 giờ qua.' },
        ],
      },
      {
        heading: 'Đọc thẻ phiên',
        points: [
          'Mỗi thẻ cho biết trạng thái, việc Claude đang làm lúc này, và các nhãn cho nơi phiên chạy, mô hình, mức nỗ lực, chế độ, tác tử, ngữ cảnh và chi phí.',
          'Khi có một yêu cầu đơn giản đang chờ, bạn có thể trả lời ngay trên thẻ. Nếu không, nhấn **Xem xét**.',
          'Chạm vào tên phiên để mở trang của phiên.',
          'Khi có nhiều tài khoản Claude, một nhãn màu ghi tên tài khoản.',
        ],
      },
      {
        heading: 'Ngay lúc này',
        intro:
          'Hàng trên cùng đếm những gì đang diễn ra: phiên **đang làm việc**, phiên **sẽ tiếp tục**, **tác tử đang chạy**, phiên **cần bạn**, và chi phí từ đầu ngày. Mỗi phần là một lối tắt.',
        points: [
          'Bên cạnh là mức dùng gói Claude của bạn, cho khung 5 giờ và cả tuần. Chạm vào để xem mọi khung và thời điểm đặt lại.',
          'Chi phí là ước tính, tính từ nửa đêm.',
        ],
      },
      {
        heading: 'Tìm một phiên',
        steps: [
          'Gõ vào ô **Tìm phiên**.',
          'Chọn **Đang chạy**, **Cần bạn** hoặc **Tất cả**.',
          'Thu hẹp kết quả bằng **Tất cả môi trường** và **Tất cả dự án** (và **Tất cả tài khoản**, khi có nhiều tài khoản).',
          'Trong **Tất cả**, nhấn **Hiện phiên đã kết thúc** để thêm các phiên trước đó; **Tải thêm** đi xa hơn về trước.',
        ],
        tips: ['Khi không có kết quả nào, **Xóa bộ lọc** hiện lại mọi thứ.'],
      },
      {
        heading: 'Chọn nhiều phiên',
        steps: [
          'Nhấn **Chọn**.',
          'Đánh dấu các phiên bạn muốn, hoặc nhấn **Chọn tất cả** để lấy mọi phiên khớp với tìm kiếm và bộ lọc.',
          'Nhấn **Xóa N…** hoặc **Di chuyển hoặc sao chép N…**.',
          'Nhấn **Xong** khi bạn đã làm xong.',
        ],
        links: [{ label: 'Xem “Di chuyển, sao chép và xóa”', path: '/help/move-data' }],
      },
      {
        heading: 'Dự án',
        intro:
          '**Dự án**, cạnh **Phiên** ở trên cùng, liệt kê mọi thư mục dự án có cuộc trò chuyện, trong mọi tài khoản: có bao nhiêu cuộc trò chuyện, bao nhiêu đang chạy, dung lượng và hoạt động gần nhất.',
        points: [
          'Menu của một dự án mở thư mục của nó trên ổ đĩa (ứng dụng máy tính), sao chép đường dẫn, di chuyển hay sao chép dự án, hoặc xóa nó.',
          'Dự án đã xóa nằm trong **Đã xóa gần đây** trong bảy ngày, kèm **Hoàn tác**.',
        ],
      },
      {
        heading: 'Tác tử',
        intro:
          'Trang **Tác tử** hiện mọi tác tử con, tác tử quy trình, shell nền và trình giám sát đang chạy, nhóm theo phiên. Trên điện thoại, nó có thẻ riêng.',
        points: [
          'Chạm vào một tác tử để đọc bản ghi riêng của nó, cập nhật trực tiếp trong lúc nó chạy. Shell hoặc trình giám sát thì hiện đầu ra của nó.',
          'Bạn có thể dừng một tác tử khi phiên của nó cho phép.',
          'Con số trên **Tác tử** đếm số tác tử đang chạy.',
        ],
        links: [
          { label: 'Mở trang Phiên', path: '/' },
          { label: 'Mở trang Tác tử', path: '/agents' },
        ],
      },
    ],
  },
  {
    id: 'session-page',
    title: 'Trang của một phiên',
    icon: 'file',
    group: 'sessions',
    summary: 'Trò chuyện, tác tử, thay đổi và thông tin của một phiên, cùng menu của nó.',
    intro:
      'Chạm vào một phiên để mở trang của nó. Phần trên cùng hiện tên, trạng thái và việc Claude đang làm lúc này. Bên dưới là bốn thẻ: **Trò chuyện**, **Tác tử**, **Thay đổi** và **Thông tin**.',
    keywords: ['bản ghi', 'cuộc trò chuyện', 'khác biệt', 'thẻ', 'trình đơn', 'trạng thái'],
    sections: [
      {
        heading: 'Phần đầu trang',
        points: [
          'Mũi tên ở góc trên bên trái đưa bạn về trang **Phiên**. Dòng dưới tên cho biết Claude đang làm gì, ví dụ công cụ nó đang chạy, và đã bao lâu.',
          'Hàng kế tiếp cho biết phiên chạy ở đâu và được kết nối thế nào (ví dụ **VS Code · Qua hook**), rồi đến mô hình, chế độ quyền và mức nỗ lực, mức dùng ngữ cảnh và chi phí.',
          'Chạm vào một nhãn để thay đổi nó. Nhãn nào không đổi được sẽ giải thích lý do khi bạn chạm vào.',
          '**⋯** mở menu của phiên.',
        ],
      },
      {
        heading: 'Ý nghĩa của trạng thái',
        terms: [
          { term: 'Đang làm việc', text: 'Claude đang xử lý một lượt.' },
          { term: 'Cần bạn', text: 'Một yêu cầu cấp quyền, câu hỏi hoặc kế hoạch đang chờ bạn.' },
          {
            term: 'Việc chạy nền',
            text: 'Lượt đã kết thúc, nhưng việc mà lượt đó bắt đầu (tác tử, shell, trình giám sát) vẫn đang chạy. Phiên vẫn nhận tin nhắn như một phiên đang rảnh.',
          },
          {
            term: 'Đã lên lịch',
            text: 'Một lần đánh thức đã hẹn hoặc một tác vụ đã lên lịch sẽ đánh thức phiên. Trang cho biết khi nào.',
          },
          { term: 'Rảnh', text: 'Đang chờ tin nhắn tiếp theo của bạn.' },
          { term: 'Đang khởi động', text: 'Claude đang khởi động.' },
          { term: 'Đã kết thúc', text: 'Phiên đã kết thúc. Bạn có thể tiếp tục nó.' },
          { term: 'Thất bại', text: 'Phiên dừng vì gặp lỗi.' },
          { term: 'Đã dừng', text: 'Phiên đã bị dừng.' },
        ],
        tips: [
          'Phiên nào tự tiếp tục sẽ ghi rõ trên trang của nó, cùng điều gì sẽ dừng nó (ví dụ đóng VS Code).',
        ],
      },
      {
        heading: 'Trò chuyện',
        points: [
          'Cuộc trò chuyện: tin nhắn của bạn, câu trả lời của Claude, các công cụ nó đã chạy và kết quả.',
          'Phiên do Mother Claude chạy hiện câu trả lời từng chữ. Phiên bắt đầu ở nơi khác hiện mỗi tin nhắn khi tin nhắn đó hoàn tất.',
          '**Tải tin nhắn cũ hơn** hiện các tin nhắn trước đó.',
          'Yêu cầu duyệt, câu hỏi và kế hoạch đang chờ hiện phía trên ô soạn tin, lần lượt từng mục.',
        ],
        links: [{ label: 'Xem “Tin nhắn và điều khiển”', path: '/help/messaging' }],
      },
      {
        heading: 'Tác tử',
        points: [
          'Tác tử con dạng cây, các lượt chạy quy trình cùng các giai đoạn của chúng, và shell nền, trình giám sát cùng đầu ra mới nhất.',
          'Chạm vào một tác tử để đọc bản ghi của nó. Bạn có thể dừng một tác tử khi phiên cho phép.',
        ],
      },
      {
        heading: 'Thay đổi',
        points: [
          'Các thay đổi git của phiên. **Chưa commit** hiện phần việc chưa được commit; **Cả nhánh** hiện mọi thứ kể từ khi nhánh tách khỏi nhánh mặc định.',
          'Hiện các tệp dạng **Danh sách** hoặc **Cây**, và thay đổi của một tệp ở dạng **Gộp** hoặc **Chia đôi**.',
          '**Làm mới** đọc lại các thay đổi.',
        ],
      },
      {
        heading: 'Thông tin',
        terms: [
          {
            term: 'Phiên',
            text: 'Tên, trạng thái, id, dự án và thư mục, đường dẫn bản ghi, nơi phiên chạy, tài khoản Claude, nhánh và pull request.',
          },
          {
            term: 'Mô hình và mức dùng',
            text: 'Mô hình, chế độ quyền, mức nỗ lực, ultracode, suy nghĩ, chế độ nhanh, chi phí và token.',
          },
          { term: 'Mức dùng gói', text: 'Gói của tài khoản mà phiên đang dùng.' },
          {
            term: 'Từ đây bạn có thể',
            text: 'Những gì dùng được với phiên này, và nút khắc phục từng giới hạn.',
          },
          {
            term: 'Thao tác',
            text: '**Xem terminal**, **Mở trong VS Code**, **Dừng**, **Khởi chạy lại**, **Gỡ bỏ**, **Gửi phím**, lệnh `claude --resume` để sao chép, và **Tiếp tục tại đây** với các tùy chọn khởi chạy.',
          },
          { term: 'Máy chủ MCP', text: 'Các máy chủ MCP của một phiên do Mother Claude chạy.' },
        ],
        tips: [
          'Cạnh đường dẫn bản ghi, **Sao chép đường dẫn** sẽ sao chép nó; trong ứng dụng máy tính, **Hiện trong Finder** hoặc **Hiện trong File Explorer** mở thư mục chứa nó.',
        ],
      },
      {
        heading: 'Menu của phiên',
        intro: 'Nhấn **⋯** ở góc trên bên phải. Menu có những mục áp dụng cho phiên này:',
        terms: [
          {
            term: 'Đổi tên…',
            text: 'Đổi tên phiên. Với phiên VS Code, tên mới chỉ hiện trong Mother Claude.',
          },
          {
            term: 'Tiếp tục tại đây',
            text: 'Tiếp quản phiên để Mother Claude chạy nó (**Tiếp tục tại đây (rẽ nhánh)** khi phiên vẫn đang chạy ở nơi khác).',
          },
          {
            term: 'Đổi tài khoản…',
            text: 'Chạy phiên bằng một tài khoản Claude khác (khi có nhiều tài khoản). **Tiếp tục trên tài khoản khác…** làm việc này cho phiên bắt đầu ở nơi khác.',
          },
          {
            term: 'Di chuyển hoặc sao chép…',
            text: 'Di chuyển hoặc sao chép cuộc trò chuyện, hoặc cả dự án của nó với **Di chuyển hoặc sao chép dự án này…**, sang một tài khoản khác hay một kho lưu trữ.',
          },
          { term: 'Xóa…', text: 'Xóa cuộc trò chuyện, có bảy ngày để hoàn tác.' },
          {
            term: 'Xem terminal',
            text: 'Những gì terminal của phiên đang hiện (phiên tmux và tác vụ nền).',
          },
          {
            term: 'Thông báo',
            text: 'Trên điện thoại: **Theo dõi**, **Mặc định** hoặc **Tắt tiếng** thông báo đẩy cho phiên này, và **Báo cho tôi khi lượt này xong** trong lúc phiên làm việc.',
          },
          {
            term: 'claude --resume …',
            text: '**Sao chép** đưa lệnh để tiếp tục phiên trong terminal vào bộ nhớ tạm.',
          },
          { term: 'Mở trong VS Code', text: 'Cho các phiên chạy trong VS Code.' },
          { term: 'Thông tin phiên', text: 'Mở thẻ **Thông tin**.' },
          {
            term: 'Dừng, Khởi chạy lại, Gỡ bỏ',
            text: 'Cho tác vụ nền. **Gỡ bỏ** cần ứng dụng máy tính, hoặc một thiết bị được phép thực hiện thao tác nguy hiểm.',
          },
        ],
      },
    ],
  },
  {
    id: 'messaging',
    title: 'Tin nhắn và điều khiển',
    icon: 'send',
    group: 'sessions',
    summary: 'Gửi tin nhắn và tệp, Gửi ngay, ngắt, và đổi mô hình, mức nỗ lực, chế độ.',
    intro:
      'Những gì bạn làm được với một phiên tùy vào việc ai đã bắt đầu nó. Phiên do Mother Claude bắt đầu (gọi là phiên **thuộc Mother Claude**) được điều khiển hoàn toàn. Phiên bắt đầu ở nơi khác được kết nối qua plugin Mother Claude hoặc tmux, hoặc được tiếp quản bằng **Tiếp tục tại đây**.',
    keywords: [
      'thuộc mother claude',
      'phiên bên ngoài',
      'ô soạn tin',
      'chỉnh hướng',
      'hàng đợi',
      'đang chờ',
      'rẽ nhánh',
      'lệnh gạch chéo',
      'đính kèm',
      'hình ảnh',
      'ảnh',
    ],
    sections: [
      {
        heading: 'Bạn điều khiển được phiên đến đâu',
        intro:
          'Mỗi phiên có một huy hiệu cho biết cách nó được kết nối. Chạm vào để xem những gì dùng được và vì sao.',
        terms: [
          {
            term: 'Điều khiển đầy đủ',
            text: 'Do Mother Claude bắt đầu (từ **Phiên mới** hoặc **Tiếp tục tại đây**). Mọi thứ hoạt động ngay, và câu trả lời hiện ra từng chữ.',
          },
          {
            term: 'Qua hook',
            text: 'Bắt đầu trong terminal, VS Code, JetBrains hay Claude Desktop và được kết nối qua plugin Mother Claude. Yêu cầu duyệt hoạt động; tin nhắn đến tay Claude ở điểm dừng kế tiếp.',
          },
          { term: 'Qua tmux', text: 'Chạy trong một khung tmux; Mother Claude gõ vào đó.' },
          {
            term: 'Thử nghiệm',
            text: 'Một tác vụ nền (`claude --bg`), được gõ vào qua terminal của nó. Có thể không hoạt động ổn định.',
          },
          {
            term: 'Chỉ xem',
            text: 'Bạn xem được nhưng không điều khiển được. **Tiếp tục tại đây** sẽ tiếp quản nó.',
          },
        ],
        links: [{ label: 'Xem “Phiên bắt đầu ở nơi khác”', path: '/help/foreign-sessions' }],
      },
      {
        heading: 'Gửi tin nhắn',
        steps: [
          'Mở phiên và ở lại thẻ **Trò chuyện**.',
          'Gõ vào ô **Nhắn cho Claude…**.',
          'Nhấn **Gửi** (hoặc Enter trên bàn phím).',
        ],
        tips: [
          'Shift+Enter thêm dòng mới. Trên điện thoại, Enter thêm dòng mới, nên hãy dùng **Gửi**. Cmd+Enter hoặc Ctrl+Enter luôn gửi đi.',
          'Bản nháp được giữ riêng cho từng phiên cho đến khi gửi đi. Nếu gửi không được, nội dung vẫn còn: **Thử lại** sẽ gửi lại.',
        ],
      },
      {
        heading: 'Gửi trong lúc Claude làm việc',
        intro:
          'Bạn không cần chờ hết lượt. Trong phiên do Mother Claude chạy, tin nhắn bạn gửi khi Claude đang làm việc sẽ được Claude đọc ngay khi bước hiện tại kết thúc (một lệnh, một lần sửa tệp), ngay trong lượt đó.',
        steps: [
          'Trong lúc Claude làm việc, gõ tin nhắn của bạn.',
          'Nhấn **Gửi**. Tin nhắn chờ phía trên ô soạn tin, kèm nhãn **sau bước hiện tại**.',
          'Để Claude đọc ngay, nhấn **Gửi ngay** trên tin nhắn đang chờ (hoặc **Gửi ngay** cạnh **Gửi** trước khi gửi).',
        ],
        points: [
          '**Gửi ngay** khiến Claude đọc tin nhắn tức thì: Claude Code dừng bước đang làm, hoặc chuyển bước đó sang chạy nền, để nhận tin nhắn của bạn.',
          'Nhấn **✕** trên một tin nhắn đang chờ để rút lại trước khi Claude đọc.',
          'Trong phiên bắt đầu ở nơi khác, tin nhắn chờ đến lần gọi công cụ kế tiếp của Claude hoặc đến cuối lượt (**đang chờ bước tiếp theo**). **Ngắt & gửi** sẽ ngắt lượt và gửi tin nhắn.',
        ],
      },
      {
        heading: 'Đính kèm ảnh và tệp',
        steps: [
          'Nhấn nút máy ảnh để thêm ảnh, hoặc nút kẹp giấy để đính kèm tệp.',
          'Chọn các tệp.',
          'Viết tin nhắn và nhấn **Gửi**.',
        ],
        points: [
          'Phiên do Mother Claude chạy nhận ảnh ngay trong tin nhắn; ảnh được thu nhỏ còn tối đa 2048 pixel.',
          'Các tệp khác, và mọi tệp gửi tới phiên bắt đầu ở nơi khác, được tải lên rồi đưa cho Claude dưới dạng đường dẫn tệp.',
          'Nhấn **✕** trên ảnh thu nhỏ để bỏ nó trước khi gửi.',
        ],
      },
      {
        heading: 'Lệnh gạch chéo và đường dẫn tệp',
        points: [
          'Gõ `/` để mở danh sách lệnh. `/model`, `/effort`, `/mode` và `/rename` mở bộ chọn tương ứng; `/clear` và `/compact` sẽ hỏi trước.',
          'Gõ `@` để điền nhanh một đường dẫn tệp trong dự án.',
          'Lệnh gạch chéo dùng được trong phiên do Mother Claude chạy và trong phiên tmux. Trong tmux, mọi lệnh trừ các lệnh thông dụng (như `/compact`, `/clear`, `/model` và `/effort`) đều cần một thiết bị được phép thực hiện thao tác nguy hiểm.',
        ],
        links: [
          {
            label: 'Xem “Phiên trong tmux”',
            path: '/help/foreign-sessions',
            fragment: 'sessions-in-tmux',
          },
        ],
      },
      {
        heading: 'Ngắt Claude',
        steps: ['Trong lúc Claude làm việc, nhấn **Ngắt** bên dưới ô soạn tin.'],
        points: [
          'Trong phiên bắt đầu ở nơi khác, nút ghi **Ngắt ở công cụ tiếp theo**: lệnh ngắt có hiệu lực ở lần gọi công cụ kế tiếp của Claude, chứ không phải giữa chừng một câu.',
          'Khi có đúng một yêu cầu cấp quyền đang chờ, **Từ chối & gửi làm phản hồi** từ chối nó và gửi nội dung bạn viết làm lý do; **Xếp hàng sau yêu cầu này** gửi tin nhắn của bạn sau yêu cầu đó.',
        ],
      },
      {
        heading: 'Đổi mô hình, mức nỗ lực hoặc chế độ',
        steps: [
          'Mở phiên.',
          'Chạm vào nhãn mô hình, chế độ hoặc mức nỗ lực ở hàng dưới tên phiên.',
          'Chọn giá trị mới.',
        ],
        terms: [
          { term: 'Hỏi trước', text: 'Hỏi trước khi sửa tệp và chạy lệnh.' },
          { term: 'Chấp nhận chỉnh sửa', text: 'Sửa tệp được thực hiện luôn; chạy lệnh thì hỏi.' },
          { term: 'Lập kế hoạch', text: 'Lập kế hoạch trước, không thay đổi gì.' },
          { term: 'Tự động', text: 'Một bộ phân loại quyết định việc gì cần đến bạn.' },
          {
            term: 'Bỏ qua quyền',
            text: 'Không bao giờ hỏi. Nguy hiểm: chỉ dùng được trong ứng dụng máy tính, hoặc trên thiết bị được phép thực hiện thao tác nguy hiểm.',
          },
        ],
        points: [
          'Mức nỗ lực đi từ **Thấp** qua **Trung bình**, **Cao** và **Rất cao** đến **Tối đa**. Ultracode và suy nghĩ có công tắc riêng trong phiên do Mother Claude chạy.',
          'Phiên bắt đầu ở nơi khác không đổi được mô hình hay mức nỗ lực qua plugin. Phiên tmux nhận `/model` và `/effort`, và các lệnh này cũng đổi giá trị mặc định của bạn cho phiên mới.',
        ],
      },
      {
        heading: 'Tiếp tục tại đây',
        intro:
          '**Tiếp tục tại đây** biến một phiên thành phiên do Mother Claude chạy, với toàn quyền điều khiển.',
        steps: [
          'Mở phiên.',
          'Nhấn **⋯**.',
          'Chọn **Tiếp tục tại đây**, hoặc **Tiếp tục tại đây (rẽ nhánh)** với phiên vẫn đang chạy ở nơi khác.',
        ],
        points: [
          'Phiên vẫn đang chạy ở nơi khác sẽ được rẽ nhánh: bạn điều khiển một bản sao với id mới, còn bản gốc vẫn tiếp tục chạy ở chỗ cũ.',
          'Phiên đã kết thúc sẽ tiếp tục ngay tại chỗ. Gõ tin nhắn vào phiên đó cũng có tác dụng như vậy: nút gửi của nó ghi **Tiếp tục**.',
          'Để chọn mô hình, mức nỗ lực hoặc chế độ trước, dùng **Tiếp tục tại đây** trong thẻ **Thông tin** và mở **Tùy chọn khởi chạy (mô hình · mức nỗ lực · chế độ)**.',
          'Nếu cuộc trò chuyện bắt đầu trên một máy tính khác, có thể bạn sẽ được yêu cầu chọn thư mục của nó trên máy này trước.',
        ],
        notes: [
          'Mỗi lúc chỉ điều khiển một cuộc trò chuyện từ một nơi: vừa gõ trong VS Code hay terminal vừa gõ ở đây sẽ tách nó làm hai.',
          'Từ điện thoại, việc này chỉ làm được trong các thư mục mà Claude Code đã tin cậy trên máy tính của bạn.',
        ],
        links: [
          {
            label: 'Xem “Tiếp tục cuộc trò chuyện từ máy tính khác”',
            path: '/help/backup',
            fragment: 'continue-a-conversation-from-another-computer',
          },
        ],
      },
    ],
  },
  {
    id: 'approvals',
    title: 'Yêu cầu duyệt và Hộp thư',
    icon: 'inbox',
    group: 'sessions',
    summary: 'Trả lời yêu cầu cấp quyền, câu hỏi và kế hoạch từ mọi phiên.',
    intro:
      'Khi Claude cần bạn, một thẻ sẽ hiện ra: phía trên ô soạn tin trên trang của phiên, trên thẻ của phiên trong danh sách, và trong **Hộp thư**, nơi gom mọi thứ đang chờ ở tất cả các phiên.',
    keywords: [
      'quyền',
      'cho phép',
      'từ chối',
      'duyệt',
      'câu hỏi',
      'kế hoạch',
      'nguy hiểm',
      'đang chờ',
    ],
    sections: [
      {
        heading: 'Hộp thư',
        points: [
          '**Cần bạn** liệt kê những gì bạn trả lời được từ đây, cũ nhất trước, kèm thời gian mỗi mục đã chờ.',
          '**Chờ tại máy tính** liệt kê những gì chỉ trả lời được ở nơi phiên đang chạy, và điều gì sẽ giúp bạn trả lời được ngay tại đây.',
          'Con số trên **Hộp thư** đếm số mục đang chờ.',
          'Khi có nhiều tài khoản Claude, bộ lọc tài khoản hiện từng tài khoản một.',
        ],
        tips: [
          'Trên bàn phím, J và K chuyển giữa các yêu cầu; A cho phép, D từ chối, F thêm phản hồi.',
        ],
        links: [{ label: 'Mở trang Hộp thư', path: '/inbox' }],
      },
      {
        heading: 'Trả lời yêu cầu cấp quyền',
        intro: 'Đọc xem Claude muốn chạy hay thay đổi gì, rồi chọn:',
        terms: [
          { term: 'Cho phép một lần', text: 'Chỉ cho lần gọi này chạy.' },
          {
            term: 'Luôn cho phép…',
            text: 'Cho phép ngay và ghi nhớ một quy tắc. Chọn nơi lưu quy tắc: **Chỉ phiên này**, hoặc một tệp cài đặt cho dự án hay cho riêng bạn.',
          },
          { term: 'Từ chối', text: 'Không cho chạy.' },
          {
            term: 'Từ chối kèm phản hồi…',
            text: 'Không cho chạy và nói với Claude nên làm gì thay vào đó. Đánh dấu **Dừng luôn lượt hiện tại** để dừng cả lượt.',
          },
        ],
        notes: [
          'Lưu quy tắc vào tệp cài đặt cần ứng dụng máy tính, hoặc một thiết bị được phép thực hiện thao tác nguy hiểm. Điện thoại không có quyền đó chỉ được đề xuất quy tắc cho riêng phiên này.',
        ],
      },
      {
        heading: 'Trả lời câu hỏi',
        steps: [
          'Chọn một câu trả lời cho mỗi câu hỏi, hoặc tự gõ câu trả lời của bạn.',
          'Nhấn **Gửi câu trả lời** (**Gửi các câu trả lời** nếu có nhiều câu hỏi).',
        ],
        points: ['**Bỏ qua** để không trả lời.'],
      },
      {
        heading: 'Xem xét kế hoạch',
        steps: [
          'Nhấn **Đọc toàn bộ kế hoạch**.',
          'Nhấn **Cho phép & tự chấp nhận chỉnh sửa** hoặc **Cho phép & xem xét từng chỉnh sửa**.',
        ],
        points: [
          '**Tiếp tục lập kế hoạch…** yêu cầu sửa kế hoạch thay vì cho phép: viết những gì cần đổi rồi nhấn **Gửi phản hồi**.',
        ],
      },
      {
        heading: 'Yêu cầu nguy hiểm',
        intro:
          'Mother Claude đánh dấu một số yêu cầu là **nguy hiểm**: ví dụ ghi vào cài đặt của Claude Code, `.mcp.json`, tệp khởi động shell hay git hook. Thẻ sẽ nói rõ lý do.',
        points: [
          'Ứng dụng máy tính có thể cho phép các yêu cầu này.',
          'Điện thoại cần **Cho phép thao tác nguy hiểm** cho mạng mà nó đang dùng, bật trong ứng dụng máy tính tại **Từ xa › Thiết bị đã ghép nối**.',
          'Thiết bị nào cũng có thể từ chối chúng.',
        ],
      },
      {
        heading: 'Phiên bắt đầu ở nơi khác',
        points: [
          'Khi đã cài điều khiển phiên bên ngoài, yêu cầu cấp quyền từ một phiên terminal hay VS Code sẽ hiện ở đây, trong khi phiên đó vẫn hiện hộp thoại của riêng nó. Ai trả lời trước thì câu trả lời đó có hiệu lực; hộp thoại còn lại sẽ biến mất.',
          'Nếu chưa cài, các yêu cầu đó hiện trong **Chờ tại máy tính**.',
          'Phiên ở chế độ **Bỏ qua quyền** gần như không hỏi gì, nên rất ít yêu cầu đến được với bạn.',
        ],
        links: [{ label: 'Xem “Phiên bắt đầu ở nơi khác”', path: '/help/foreign-sessions' }],
      },
      {
        heading: 'Nhận thông báo khi bạn đi vắng',
        points: [
          'Thông báo đẩy báo cho điện thoại của bạn về yêu cầu, câu hỏi, kế hoạch và các lượt đã xong, kể cả khi ứng dụng đang đóng.',
          'Yêu cầu từ tiện ích trình duyệt và HTTP API cũng hiện ở đây, dưới dạng các thẻ giống vậy.',
        ],
        links: [
          {
            label: 'Xem “Dùng trên điện thoại”',
            path: '/help/phone',
            fragment: 'push-notifications',
          },
        ],
      },
    ],
  },
  {
    id: 'new-session',
    title: 'Bắt đầu phiên mới',
    icon: 'plus',
    group: 'sessions',
    summary: 'Bắt đầu một phiên do Mother Claude chạy, điều khiển đầy đủ từ mọi thiết bị.',
    intro:
      'Phiên bắt đầu từ trang **Phiên mới** thuộc Mother Claude: yêu cầu duyệt, câu hỏi, lệnh ngắt và đổi mô hình đều dùng được từ máy tính này và từ điện thoại của bạn.',
    keywords: ['bắt đầu', 'khởi chạy', 'tạo', 'tạo phiên', 'dự án', 'lời nhắc đầu tiên'],
    sections: [
      {
        heading: 'Bắt đầu một phiên',
        steps: [
          'Mở trang **Phiên mới**.',
          'Chọn một dự án trong **Dự án**, hoặc gõ một đường dẫn vào **Hoặc thư mục bất kỳ trên máy tính này** (ứng dụng máy tính).',
          'Nếu có nhiều tài khoản Claude, kiểm tra **Chạy bằng tài khoản**.',
          'Trong **Tùy chọn**, chọn **Mô hình**, **Mức nỗ lực** và **Quyền**, hoặc để **Mặc định** để dùng cài đặt Claude của riêng bạn.',
          'Tùy chọn: viết **Lời nhắc đầu tiên**, và nhấn **Thêm ảnh** để đính kèm ảnh.',
          'Nhấn **Bắt đầu phiên**.',
        ],
        tips: [
          'Ảnh có thể là PNG, JPEG, GIF hoặc WebP: tối đa 10 ảnh, mỗi ảnh 5 MB.',
          '**Tùy chỉnh…** trong **Mô hình** nhận bất kỳ id mô hình nào.',
        ],
        links: [{ label: 'Mở trang Phiên mới', path: '/new' }],
      },
      {
        heading: 'Phiên chạy bằng tài khoản nào',
        points: [
          'Khi có nhiều tài khoản, **Chạy bằng tài khoản** chọn sẵn tài khoản của dự án, hoặc tài khoản mặc định cho phiên mới.',
          'Tài khoản không chạy được ở đây (đã đăng xuất, hoặc chưa có token trên máy tính này) sẽ bị làm mờ.',
        ],
        links: [{ label: 'Xem “Tài khoản Claude”', path: '/help/accounts' }],
      },
      {
        heading: 'Từ điện thoại',
        points: [
          'Chọn một trong các dự án gần đây: gõ thư mục bất kỳ chỉ làm được trên ứng dụng máy tính.',
          'Phiên mới chỉ bắt đầu được trong thư mục mà Claude Code đã tin cậy trên máy tính của bạn (đã mở ở đó một lần). Nếu không, bạn sẽ thấy `untrusted_cwd`: hãy mở thư mục đó trong Claude Code trên máy tính một lần.',
          'Chỉ chọn được **Bỏ qua quyền** trong ứng dụng máy tính, hoặc trên thiết bị được phép thực hiện thao tác nguy hiểm.',
        ],
      },
    ],
  },

  // ────────────────────────────────────────────── Điện thoại & từ xa
  {
    id: 'phone',
    title: 'Dùng trên điện thoại',
    icon: 'phone',
    group: 'remote',
    summary:
      'Ghép nối điện thoại qua Wi-Fi, tin cậy chứng chỉ, thêm vào Màn hình chính, nhận thông báo đẩy.',
    intro:
      'Điện thoại của bạn mở cùng bảng điều khiển như ứng dụng máy tính, do máy tính của bạn cung cấp. Ghép nối một lần qua Wi-Fi ở nhà; để truy cập từ mọi nơi, xem phần Truy cập qua internet.',
    keywords: [
      'ghép nối',
      'mã qr',
      'mã ghép nối',
      'mạng nội bộ',
      'chứng chỉ',
      'thông báo',
      'thông báo đẩy',
      'màn hình chính',
      'di động',
    ],
    sections: [
      {
        heading: 'Ghép nối điện thoại trên cùng Wi-Fi',
        steps: [
          'Trong ứng dụng máy tính, mở trang **Từ xa**.',
          'Trong **Ghép nối thiết bị**, chọn **Cùng Wi-Fi**.',
          'Nhấn **Hiện mã ghép nối**. Một mã QR và một mã 12 ký tự sẽ hiện ra.',
          'Quét mã QR bằng camera của điện thoại.',
          'Điện thoại mở ra các phiên của bạn, đã ghép nối xong.',
        ],
        tips: [
          'Không quét được? Trên điện thoại, mở địa chỉ hiện bên dưới mã, gõ mã vào trang **Ghép nối thiết bị này**, rồi nhấn **Ghép nối**.',
          'Mỗi mã chỉ dùng được một lần và hết hạn sau 5 phút. Nếu máy tính có nhiều địa chỉ mạng, hãy chọn địa chỉ mà điện thoại kết nối được trong **Địa chỉ**.',
        ],
        notes: [
          'Mã **Cùng Wi-Fi** chỉ dùng được trong mạng của chính bạn. Chỉ ghép nối trên mạng mà bạn kiểm soát.',
        ],
        links: [{ label: 'Mở trang Từ xa', path: '/remote' }],
      },
      {
        heading: 'Tin cậy chứng chỉ của máy tính',
        intro:
          'Ở nhà, điện thoại kết nối qua HTTPS bằng một chứng chỉ do máy tính của bạn tạo ra, nên trình duyệt sẽ cảnh báo một lần (“không riêng tư”). Tin cậy chứng chỉ này sẽ bật cập nhật trực tiếp trên iPhone, ứng dụng Màn hình chính và thông báo đẩy.',
        steps: [
          'Trên trang Ghép nối của điện thoại, mở **Tin cậy máy Mac này (khuyên dùng)**.',
          'iPhone hoặc iPad: chạm **Tải hồ sơ về**, rồi cài đặt hồ sơ trong Cài đặt › Cài đặt chung › VPN & Quản lý thiết bị (Settings › General › VPN & Device Management).',
          'iPhone hoặc iPad: trong Cài đặt › Cài đặt chung › Giới thiệu › Cài đặt tin cậy chứng nhận (Settings › General › About › Certificate Trust Settings), bật tin cậy hoàn toàn cho “Mother Claude”.',
          'Android: chạm **Tải chứng chỉ của máy Mac về** và cài đặt nó trong Cài đặt › Bảo mật › Cài đặt chứng chỉ › Chứng chỉ CA (Settings › Security › Install a certificate › CA certificate).',
          'Tải lại trang.',
        ],
        points: [
          'Trang Từ xa hiện dấu vân tay của chứng chỉ, để bạn so sánh với dấu vân tay mà điện thoại hiện.',
          'Chứng chỉ này chỉ bảo chứng được cho các địa chỉ riêng, nên không thể dùng nó để giả mạo một trang web công khai.',
        ],
      },
      {
        heading: 'Thêm vào Màn hình chính của iPhone',
        intro:
          'Ứng dụng thêm vào Màn hình chính giữ dữ liệu riêng, tách biệt với Safari, nên nó được ghép nối riêng.',
        steps: [
          'Trong Safari, chạm Chia sẻ › Thêm vào MH chính (Share › Add to Home Screen).',
          'Trong một thẻ Safari đã ghép nối, mở **Cài đặt** và nhấn **Thiết lập ứng dụng Màn hình chính**.',
          'Mở ứng dụng từ Màn hình chính.',
          'Gõ mã vào trang Ghép nối của ứng dụng và nhấn **Ghép nối**.',
        ],
        tips: [
          'Mã chỉ dùng được một lần, trong 5 phút, và chỉ cho thiết bị này. Cùng mã đó cũng ghép nối được điện thoại này ở một địa chỉ khác, chẳng hạn địa chỉ đường hầm của bạn.',
        ],
      },
      {
        heading: 'Thông báo đẩy',
        intro:
          'Thông báo đẩy đến được điện thoại ngay cả khi ứng dụng đang đóng. Nó cần một địa chỉ HTTPS đáng tin cậy: địa chỉ đường hầm của bạn, hoặc địa chỉ ở nhà sau khi đã tin cậy chứng chỉ. Trên iPhone, nó chỉ hoạt động trong ứng dụng Màn hình chính.',
        steps: [
          'Trên điện thoại, mở **Cài đặt**.',
          'Trong **Thông báo**, nhấn **Bật** cạnh **Thông báo đẩy**.',
          'Cho phép thông báo khi trình duyệt hỏi.',
          'Chọn các sự kiện bạn muốn: **Yêu cầu cấp quyền**, **Câu hỏi**, **Kế hoạch**, **Đã xong**, **Lỗi**, **Khi Claude hỏi** và **Đang chờ bạn**.',
          'Nhấn **Gửi thử**.',
        ],
        points: [
          '**Hiện chi tiết** thêm lệnh hoặc câu hỏi vào thông báo.',
          'Cho riêng một phiên: mở menu **⋯** của phiên và chọn **Theo dõi** hoặc **Tắt tiếng**, hoặc **Báo cho tôi khi lượt này xong**.',
          'Không có thông báo nào được đẩy tới thiết bị đang mở ứng dụng trước mặt bạn, và nhiều thông báo dồn dập từ một phiên sẽ được gộp lại.',
          'Ứng dụng máy tính không cần thông báo đẩy: nó hiện mọi thứ trực tiếp.',
        ],
        notes: ['Máy tính của bạn gửi mọi thông báo, nên máy phải đang thức và có mạng.'],
      },
      {
        heading: 'Điện thoại làm được gì',
        points: [
          'Xem mọi phiên, trả lời yêu cầu duyệt, câu hỏi và kế hoạch, gửi tin nhắn và ảnh, bắt đầu phiên và tiếp tục phiên.',
          'Ghép nối thiết bị, thay đổi quyền truy cập từ xa và cài đặt plugin chỉ làm được trong ứng dụng máy tính.',
          'Thao tác nguy hiểm cần được cấp quyền cho thiết bị này trong ứng dụng máy tính (**Từ xa › Thiết bị đã ghép nối**).',
        ],
      },
      {
        heading: 'Hủy ghép nối điện thoại này',
        steps: [
          'Trên điện thoại, mở **Cài đặt**.',
          'Trong **Kết nối**, nhấn **Hủy ghép nối thiết bị này**.',
          'Nhấn tiếp **Nhấn lần nữa để hủy ghép nối**.',
        ],
        points: [
          'Thông báo đẩy của điện thoại dừng lại và ứng dụng máy tính quên thiết bị này. Bạn cũng có thể thu hồi nó từ **Từ xa › Thiết bị đã ghép nối**.',
        ],
      },
    ],
  },
  {
    id: 'internet',
    title: 'Truy cập qua internet và thiết bị',
    icon: 'shield',
    group: 'remote',
    summary: 'Kết nối tới máy tính qua đường hầm, cho phép thiết bị, và quản lý quyền của chúng.',
    intro:
      'Khi không ở nhà, điện thoại kết nối tới máy tính của bạn qua một đường hầm do bạn chạy (Tailscale, ngrok hoặc Cloudflare). Mother Claude không bao giờ tự mở cổng ra internet, và truy cập qua internet tắt cho đến khi bạn bật nó.',
    keywords: ['đường hầm', 'từ xa', 'vắng nhà', 'thu hồi', 'cấp quyền', 'cổng', 'thiết bị'],
    sections: [
      {
        heading: 'Bật truy cập qua internet',
        where: 'desktop',
        steps: [
          'Mở trang **Từ xa**.',
          'Trong **Truy cập qua internet**, bật **Cho phép truy cập qua đường hầm**.',
          'Trong **Khởi động đường hầm**, sao chép lệnh cho loại đường hầm bạn dùng.',
          'Chạy lệnh đó trong terminal trên máy tính của bạn.',
          'Chờ địa chỉ hiện trong **Địa chỉ công khai**.',
        ],
        terms: [
          {
            term: 'Tailscale Serve',
            text: 'Khuyên dùng: chỉ các thiết bị đã đăng nhập vào tailnet của bạn mới kết nối được.',
          },
          { term: 'ngrok', text: 'Một địa chỉ ngẫu nhiên, hoặc tên miền cố định của bạn.' },
          {
            term: 'Cloudflare quick tunnel',
            text: 'Mỗi lần chạy có một địa chỉ mới, nên điện thoại phải ghép nối lại.',
          },
          { term: 'Tailscale Funnel', text: 'Công khai: ai cũng truy cập được địa chỉ này.' },
        ],
        tips: [
          'Mother Claude tự tìm địa chỉ của ngrok, Cloudflare quick tunnel và Tailscale. Với đường hầm khác, gõ địa chỉ `https://` của nó vào **Hoặc tự nhập**.',
        ],
        links: [{ label: 'Mở trang Từ xa', path: '/remote' }],
      },
      {
        heading: 'Ghép nối điện thoại qua internet',
        where: 'desktop',
        steps: [
          'Trong **Ghép nối thiết bị**, chọn **Internet**.',
          'Nhấn **Hiện mã ghép nối**.',
          'Quét mã bằng điện thoại (ví dụ khi đang dùng dữ liệu di động). Điện thoại sẽ hiện 4 chữ số.',
          'Trong ứng dụng máy tính, nhấn **Cho phép…** trên thẻ **Đang chờ bạn cho phép**.',
          'Gõ 4 chữ số mà điện thoại hiện.',
        ],
        points: [
          'Không ai vào được qua internet nếu bạn không gõ các chữ số đó trên máy tính của mình. Nhập sai ba lần thì yêu cầu bị từ chối.',
          'Điện thoại đã ghép nối ở nhà sẽ chưa được ghép nối ở địa chỉ đường hầm, vì trình duyệt lưu dữ liệu riêng cho từng địa chỉ. Ở nơi điện thoại đã ghép nối, nhấn **Cài đặt › Thiết lập ứng dụng Màn hình chính** để lấy mã, rồi gõ mã đó vào trang Ghép nối ở địa chỉ mới.',
        ],
      },
      {
        heading: 'Địa chỉ Tailscale',
        points: [
          '**Lắng nghe cả trên Tailscale** đưa bảng điều khiển lên cả địa chỉ Tailscale của máy tính bạn (100.x.y.z).',
          'Yêu cầu đến địa chỉ đó được tính là từ internet: mã **Cùng Wi-Fi** bị từ chối, và quyền dành cho internet được áp dụng.',
        ],
      },
      {
        heading: 'Tắt truy cập qua internet',
        steps: ['Trong **Truy cập qua internet**, tắt **Cho phép truy cập qua đường hầm**.'],
        points: [
          'Thao tác này đóng cổng đường hầm và cắt mọi kết nối đã đi qua nó.',
          'Hãy trỏ đường hầm vào cổng 6726, như các lệnh trên trang Từ xa. Một đường hầm trỏ vào cổng 6725 sẽ vẫn hoạt động sau khi bạn tắt mục này.',
        ],
      },
      {
        heading: 'Quản lý thiết bị đã ghép nối',
        where: 'desktop',
        intro:
          '**Từ xa › Thiết bị đã ghép nối** liệt kê mọi thiết bị cùng nền tảng của nó, lần kết nối gần nhất (khi nào, bằng cách nào) và các kết nối đang mở.',
        terms: [
          { term: 'Biểu tượng bút chì', text: 'Đổi tên thiết bị.' },
          {
            term: 'Thu hồi',
            text: 'Nhấn hai lần (**Nhấn lần nữa để thu hồi**). Thiết bị bị ngắt kết nối ngay và phải ghép nối lại.',
          },
          {
            term: 'Cho phép thao tác nguy hiểm (LAN)',
            text: 'Cho thiết bị làm những việc vốn chỉ dành cho ứng dụng máy tính, khi nó đang ở trong mạng nhà bạn.',
          },
          {
            term: '… qua internet',
            text: 'Tương tự, nhưng qua đường hầm hoặc địa chỉ Tailscale. Hãy để tắt trừ khi bạn thật sự cần: khi bật, một chiếc điện thoại bị đánh cắp có thể chạy bất cứ thứ gì trên máy tính của bạn.',
          },
        ],
        points: [
          'Thao tác nguy hiểm gồm: chế độ bỏ qua quyền, lưu quy tắc vào cài đặt, gỡ bỏ phiên, gửi phím thô vào terminal, và cho phép các lệnh bị đánh dấu nguy hiểm. Mặc định, chúng bị tắt với mọi thiết bị.',
          'Thiết bị không dùng trong 30 ngày sẽ tự động bị hủy ghép nối.',
          'Mỗi thiết bị mới, và mỗi thao tác nguy hiểm mà một thiết bị thực hiện, đều hiện thông báo trong ứng dụng máy tính.',
        ],
      },
      {
        heading: 'Cảnh báo trên trang Từ xa',
        terms: [
          {
            term: 'Trình kiểm tra của ngrok đang ghi lại',
            text: 'Khởi động lại ngrok với `--inspect=false`, như trang Từ xa hướng dẫn.',
          },
          {
            term: 'Địa chỉ của quick tunnel thay đổi',
            text: 'Điện thoại phải ghép nối lại và thông báo đẩy ngừng hoạt động. Hãy dùng Tailscale, tên miền cố định của ngrok hoặc một đường hầm Cloudflare có tên.',
          },
          {
            term: 'Tailscale Funnel là công khai',
            text: 'Với thiết bị của riêng bạn, hãy dùng Serve.',
          },
          {
            term: 'Một đường hầm trỏ vào cổng API',
            text: 'Ai tìm ra địa chỉ đó cũng có thể thử điều khiển Claude, nên chế độ không cần token của API bị tắt cho đến khi đường hầm đó dừng.',
          },
          { term: 'Đường hầm TCP thô', text: 'Hãy dùng đường hầm HTTP thay thế.' },
        ],
      },
    ],
  },
  {
    id: 'foreign-sessions',
    title: 'Phiên bắt đầu ở nơi khác',
    icon: 'plug',
    group: 'remote',
    summary:
      'Trả lời và nhắn tin cho phiên terminal, VS Code và JetBrains bằng plugin, hoặc qua tmux.',
    intro:
      'Mother Claude hiện mọi phiên Claude Code trên máy tính của bạn, bất kể thứ gì đã bắt đầu chúng, và bạn có thể xem chúng ngay. Để trả lời và nhắn tin cho các phiên bạn bắt đầu trong terminal, VS Code, JetBrains hay Claude Desktop, hãy cài đặt điều khiển phiên bên ngoài.',
    keywords: [
      'phiên bên ngoài',
      'điều khiển phiên bên ngoài',
      'dòng lệnh',
      'tải lại plugin',
      'tác vụ nền',
      'gỡ cài đặt',
    ],
    sections: [
      {
        heading: 'Cài đặt điều khiển phiên bên ngoài',
        where: 'desktop',
        steps: [
          'Mở trang **Từ xa**.',
          'Trong **Điều khiển phiên bên ngoài**, nhấn **Cài đặt điều khiển phiên bên ngoài**.',
          'Chờ đến khi **Trạng thái** ghi **đã cài đặt**.',
          'Trong mỗi phiên đã chạy từ trước, gõ `/reload-plugins` (hoặc khởi động lại phiên).',
        ],
        points: [
          'Thao tác này cài plugin Mother Claude bằng chính lệnh `claude plugin install` của Claude Code. `settings.json` và các hook riêng của bạn không bao giờ bị động đến.',
          'Phiên mới tự báo cáo về. Trang Từ xa đếm **Phiên đang báo cáo**, và liệt kê trong **Cần tải lại** các phiên đã bắt đầu trước khi cài.',
          'Khi Mother Claude đóng, các phiên của bạn hoạt động y như trước.',
        ],
        notes: [
          'Nút bị làm mờ khi không tìm thấy lệnh `claude` của Claude Code. Hãy cài Claude Code, hoặc mở ứng dụng từ một shell mà lệnh `claude` chạy được.',
        ],
        links: [
          { label: 'Mở mục Điều khiển phiên bên ngoài', path: '/remote', fragment: 'foreign' },
        ],
      },
      {
        heading: 'Những gì làm được qua plugin',
        points: [
          'Cho phép yêu cầu cấp quyền, trả lời câu hỏi và kế hoạch. Hộp thoại riêng của phiên vẫn còn: ai trả lời trước thì câu trả lời đó có hiệu lực.',
          'Gửi tin nhắn. Tin nhắn đến khi Claude gọi công cụ kế tiếp hoặc khi hết lượt, và phiên đang rảnh sẽ được đánh thức để đọc. Tin nhắn đến được Claude nhưng không hiện trong cửa sổ riêng của phiên; chỉ câu trả lời của Claude hiện ở đó.',
          'Ngắt. Lệnh ngắt có hiệu lực ở lần gọi công cụ kế tiếp.',
          'Không đổi được mô hình, mức nỗ lực hay dùng lệnh gạch chéo theo cách này, còn chế độ quyền chỉ đổi được cùng với một lần cho phép (ví dụ một kế hoạch). **Tiếp tục tại đây** cho bạn tất cả những thứ đó.',
        ],
      },
      {
        heading: 'Phiên trong tmux',
        points: [
          'Mother Claude gõ vào khung tmux của phiên, nên tin nhắn, lệnh gạch chéo và phím Esc hoạt động như thể bạn tự gõ.',
          'Nó không gõ khi đang có hộp thoại mở, nên không bao giờ vô tình trả lời hộp thoại đó. Hãy trả lời hộp thoại, hoặc mở **Xem terminal**.',
          'Nếu chưa có plugin, yêu cầu duyệt ở đó được trả lời bằng phím từ màn hình terminal (**Gửi phím** trong thẻ **Thông tin**).',
          '`/compact`, `/clear`, `/context`, `/usage`, `/cost`, `/status`, `/todos`, `/model`, `/effort`, `/rename` và `/btw` dùng được từ mọi thiết bị. Các lệnh khác, phím và dấu `!` ở đầu cần một thiết bị được phép thực hiện thao tác nguy hiểm.',
        ],
      },
      {
        heading: 'Tác vụ nền',
        points: [
          'Tác vụ bắt đầu bằng `claude --bg` có thể được dừng, khởi chạy lại và gỡ bỏ từ menu **⋯** của nó hoặc từ thẻ **Thông tin**.',
          '**Xem terminal** hiện những gì tác vụ in ra.',
          'Gõ vào một tác vụ vẫn là tính năng thử nghiệm và cần một thiết bị được phép thực hiện thao tác nguy hiểm.',
        ],
      },
      {
        heading: 'Tiếp quản một phiên',
        intro: 'Để điều khiển hoàn toàn bất kỳ phiên nào, hãy dùng **Tiếp tục tại đây**.',
        points: [
          'Phiên vẫn đang chạy ở nơi khác sẽ được rẽ nhánh thành một bản sao để bạn điều khiển tại đây; bản gốc vẫn tiếp tục chạy.',
          'Phiên đã kết thúc sẽ tiếp tục ngay tại chỗ.',
        ],
        links: [
          { label: 'Xem “Tiếp tục tại đây”', path: '/help/messaging', fragment: 'continue-here' },
        ],
      },
      {
        heading: 'Gỡ cài đặt',
        where: 'desktop',
        steps: [
          'Mở **Từ xa › Điều khiển phiên bên ngoài**.',
          'Nhấn **Gỡ cài đặt**.',
          'Nhấn tiếp **Nhấn lần nữa để gỡ cài đặt**.',
        ],
        points: [
          'Các hook mà phiên đang chạy đã nạp sẽ ngừng hoạt động trước; sau đó Claude Code gỡ plugin.',
        ],
      },
    ],
  },
];

/** Vietnamese Help topics (docs/I18N.md "Help"), by topic id. */
export const VI_TOPICS_A: Partial<Record<string, HelpTopicSource>> = Object.fromEntries(
  TOPICS.map((t) => [t.id, t]),
);
