import { HelpTopicSource } from '../help-model';

/** Vietnamese Help topics (docs/I18N.md "Help"), by topic id. */
export const VI_TOPICS_B: Partial<Record<string, HelpTopicSource>> = {
  // ────────────────────────────────────────────────── Accounts and data
  accounts: {
    id: 'accounts',
    title: 'Tài khoản Claude',
    icon: 'user',
    group: 'data',
    summary:
      'Chạy phiên bằng nhiều tài khoản Claude, xem mỗi gói còn lại bao nhiêu, và đổi sang tài khoản khác khi một tài khoản chạm giới hạn.',
    intro:
      'Mother Claude có thể chạy phiên bằng nhiều tài khoản Claude cùng lúc. Khi một tài khoản chạm giới hạn, bạn làm tiếp bằng tài khoản khác. **Tài khoản** có ba thẻ: **Tài khoản**, **Mức dùng** và **Thay đổi gần đây**; các bản sao lưu của chúng nằm ở **Bản sao lưu**. Khi chỉ có tài khoản Mặc định, ứng dụng không hiện nhãn hay ô chọn tài khoản nào.',
    keywords: [
      'gói đăng ký',
      'giới hạn',
      'đăng nhập',
      'đăng nhập lại',
      'đổi tài khoản',
      'chuyển tài khoản',
      'nhiều tài khoản',
      'thư mục',
      'cho phép sao chép',
      'mức dùng gói',
      'hạn mức',
      'giới hạn tốc độ',
      'plugin hooks',
    ],
    sections: [
      {
        heading: 'Ba loại tài khoản',
        terms: [
          {
            term: 'Mặc định',
            text: 'Tài khoản mà Claude Code trên máy tính này đang dùng. Bạn có thể đổi tên, nhưng không gỡ bỏ được.',
          },
          {
            term: 'Tài khoản token',
            text: 'Một gói đăng ký khác, làm việc trên cùng các dự án và cuộc trò chuyện (**Dùng chung dữ liệu, tài khoản khác**, nên dùng). Mother Claude lấy token của nó bằng `claude setup-token`.',
          },
          {
            term: 'Tài khoản thư mục',
            text: 'Một thiết lập Claude Code riêng (**Thư mục riêng**) với cài đặt, plugin, máy chủ MCP, cuộc trò chuyện và thông tin đăng nhập của riêng nó, ví dụ `~/.claude-work`.',
          },
        ],
        tips: [
          'Chọn tài khoản token để dùng chung mọi thứ, hoặc tài khoản thư mục để tách hẳn công việc và việc cá nhân.',
        ],
        links: [{ label: 'Mở Tài khoản', path: '/accounts' }],
      },
      {
        heading: 'Tài khoản, từng thẻ một',
        terms: [
          {
            term: 'Tài khoản',
            text: 'Mỗi tài khoản với loại, trạng thái đăng nhập, các phiên, mức dùng gói và dòng **Phiên bắt đầu ở nơi khác** của nó; tiếp theo là **Mặc định**, **Theo dự án**, **Sao chép để Tiếp tục trên** và các mục **Chuyển và sao chép** mới nhất (**Xem tất cả** mở **Thay đổi gần đây**).',
          },
          {
            term: 'Mức dùng',
            text: 'Gói của mỗi tài khoản còn lại bao nhiêu, mức dùng được đọc thường xuyên đến đâu, và các giới hạn tốc độ mà các nhóm gặp.',
          },
          {
            term: 'Bản sao lưu',
            text: 'Bản sao lưu được mã hóa các cuộc trò chuyện của từng tài khoản, việc kéo về từ các máy tính khác của bạn, kho lưu trữ và cặp thư mục.',
          },
          {
            term: 'Thay đổi gần đây',
            text: 'Mọi lần di chuyển, sao chép, sao lưu, khôi phục, kéo về và xóa, mới nhất trước, kèm **Hoàn tác** khi còn có thể.',
          },
        ],
        links: [
          { label: 'Xem “Bản sao lưu”', path: '/help/backup' },
          { label: 'Xem “Di chuyển, sao chép và xóa”', path: '/help/move-data' },
        ],
      },
      {
        heading: 'Thêm tài khoản token',
        where: 'desktop',
        steps: [
          'Mở **Tài khoản** và nhấn **Thêm tài khoản**.',
          'Chọn **Dùng chung dữ liệu, tài khoản khác** rồi nhấn **Tiếp**.',
          'Điền **Tên** và chọn **Màu**, rồi nhấn **Thêm tài khoản**.',
          'Nhấn **Đăng nhập để lấy token**.',
          'Chọn nơi mở trang đăng nhập: một cửa sổ Chrome ẩn danh (nên dùng), **Mở trong trình duyệt**, **Mở trong Chrome** hoặc **Sao chép liên kết**.',
          'Đăng nhập bằng tài khoản Claude bạn muốn dùng, rồi cho phép.',
          'Dán mã mà trang hiện ra vào **Mã từ trình duyệt** và nhấn **Gửi mã**.',
          'Khi thấy báo token đã được lưu, nhấn **Xong**.',
        ],
        tips: [
          'Cửa sổ ẩn danh giúp tránh việc một trình duyệt đã đăng nhập sẵn tài khoản Claude khác cho phép nhầm tài khoản.',
          'Đã có token? Nhấn **Tôi đã có token — dán vào**.',
        ],
      },
      {
        heading: 'Thêm tài khoản thư mục',
        where: 'desktop',
        steps: [
          'Nhấn **Thêm tài khoản**.',
          'Chọn **Thư mục riêng** rồi nhấn **Tiếp**.',
          'Điền **Tên**. **Thư mục** đã gợi ý sẵn một thư mục; nếu thư mục chưa có, nó sẽ được tạo mới.',
          'Nhấn **Thêm tài khoản**.',
          'Nhấn **Đăng nhập ngay**. Trình duyệt mặc định của bạn sẽ mở trang đăng nhập.',
          'Đăng nhập, rồi dán mã vào **Mã từ trình duyệt** và nhấn **Gửi mã**.',
          'Nhấn **Xong**.',
        ],
        tips: [
          'Nếu trình duyệt mở nhầm tài khoản Claude, hãy dùng cửa sổ Chrome ẩn danh hoặc **Sao chép liên kết**.',
        ],
      },
      {
        heading: 'Đăng nhập lại',
        where: 'desktop',
        points: [
          'Nhấn **Đăng nhập** ở dòng của tài khoản, hoặc **Lấy token** / **Token mới** với tài khoản token. Trong **Quản lý** cũng có **Đăng nhập lại** và **Kiểm tra lại đăng nhập**.',
          'Mỗi lúc chỉ chạy một lần đăng nhập. **Hủy** sẽ dừng lần đó.',
          'Nếu lần này bạn đăng nhập vào một tài khoản Claude khác với lần trước, hoặc trùng với tài khoản Claude của một tài khoản khác, sẽ có cảnh báo. Nhấn **Giữ** nếu đúng là vậy, hoặc **Đăng nhập lại**.',
        ],
        notes: [
          'Token và thông tin đăng nhập chỉ nằm trên máy tính đã tạo ra chúng. Trên máy tính khác, tài khoản cần đăng nhập riêng.',
        ],
      },
      {
        heading: 'Phiên bắt đầu ở nơi khác, theo tài khoản',
        points: [
          'Dòng của mỗi tài khoản trên trang **Tài khoản** cho biết Mother Claude có trả lời và nhắn tin được cho các phiên bạn bắt đầu dưới tài khoản đó trong terminal, VS Code hay JetBrains không: **Phiên bắt đầu ở nơi khác: đang bật** hoặc **Phiên bắt đầu ở nơi khác: đang tắt**.',
          'Trong ứng dụng máy tính, dòng đó mở **Phiên bắt đầu ở nơi khác**, nơi cài và gỡ plugin cho mọi tài khoản.',
          'Tài khoản thư mục giữ plugin trong thư mục riêng của nó, nên nó có một dòng riêng ở đó.',
          'Tài khoản token không có dòng này: các phiên của nó dùng plugin của tài khoản Mặc định.',
        ],
        links: [
          { label: 'Mở Phiên bắt đầu ở nơi khác', path: '/elsewhere' },
          { label: 'Xem “Phiên bắt đầu ở nơi khác”', path: '/help/foreign-sessions' },
        ],
      },
      {
        heading: 'Chọn tài khoản cho từng việc',
        steps: [
          'Mở **Tài khoản**.',
          'Ở **Mặc định**, đặt **Phiên mới chạy bằng** và **API chạy bằng**.',
          'Cho riêng một dự án: ở **Theo dự án**, nhập thư mục của nó vào **Thư mục dự án**, chọn **Tài khoản** và nhấn **Thêm**.',
        ],
        points: [
          'Thư mục con trong dự án cũng được tính; thư mục khớp dài nhất sẽ được ưu tiên.',
          '**Phiên mới** sẽ bắt đầu bằng tài khoản đó, và bạn có thể đổi ngay tại đó ở **Chạy dưới tài khoản**.',
        ],
      },
      {
        heading: 'Chuyển một phiên sang tài khoản khác',
        intro:
          'Với phiên do Mother Claude chạy, cuộc trò chuyện vẫn tiếp diễn như cũ, ngay trên trang đó.',
        steps: [
          'Mở menu **⋮** của phiên.',
          'Chọn **Đổi tài khoản…**.',
          'Chọn tài khoản.',
          'Nếu Claude đang làm việc, chọn **Sau lượt này** hoặc **Ngay** ở **Khi nào**.',
          'Nhấn nút **Chuyển sang …** (nút có ghi tên tài khoản).',
        ],
        points: [
          'Lần chuyển đang chờ hết lượt sẽ hiện một dòng kèm nút **Hủy**.',
          'Các tác vụ nền của phiên sẽ dừng. Nếu tài khoản kia có thư mục riêng, cuộc trò chuyện được chuyển sang đó trước (lần đầu tiên, ứng dụng sẽ hỏi: **Cho phép và tiếp tục**).',
          'Phiên bắt đầu ở nơi khác không thể đổi tài khoản khi nó vẫn đang chạy ở đó: khi ấy, nút này tiếp tục phiên trên tài khoản kia.',
        ],
      },
      {
        heading: 'Tiếp tục trên tài khoản khác',
        intro: 'Bạn nhận một bản sao để điều khiển bằng tài khoản kia; bản gốc vẫn giữ nguyên.',
        steps: [
          'Mở menu **⋮** của phiên.',
          'Chọn **Tiếp tục trên tài khoản khác…**.',
          'Chọn **Tài khoản**.',
          'Nhấn nút **Tiếp tục trên …** (nút có ghi tên tài khoản).',
        ],
        points: [
          'Lần sao chép đầu tiên vào một tài khoản có thư mục riêng sẽ hỏi trước: **Sao chép và tiếp tục**, hoặc **Để sau**.',
        ],
      },
      {
        heading: 'Cho phép sao chép giữa hai tài khoản',
        steps: [
          'Mở **Tài khoản** và tìm **Sao chép để Tiếp tục trên**.',
          'Ở **Cho phép sao chép**, chọn một cặp (từ → đến).',
          'Nhấn **Cho phép**.',
        ],
        points: [
          'Mỗi cặp chỉ có tác dụng theo một chiều. **Thu hồi** sẽ rút lại.',
          'Quyền này áp dụng cho các bản sao mà **Tiếp tục trên** tạo ra và các lần di chuyển mà **Chuyển tài khoản** thực hiện.',
        ],
      },
      {
        heading: 'Khi một tài khoản chạm giới hạn',
        points: [
          'Danh sách Phiên và trang của phiên sẽ báo điều này, và gợi ý một tài khoản khác còn dư hạn mức: **Chuyển sang** tài khoản đó, **Tiếp tục trên** tài khoản đó, hoặc **Chọn một tài khoản**.',
          '**Để sau** sẽ ẩn gợi ý cho đến khi giới hạn được đặt lại.',
          'Mức dùng gói của từng tài khoản hiện ở dòng của nó trên trang **Tài khoản**, và đầy đủ ở thẻ **Mức dùng**. Mức dùng của tài khoản token chỉ hiện sau khi đã có phiên chạy bằng tài khoản đó.',
        ],
      },
      {
        heading: 'Mỗi gói còn lại bao nhiêu',
        points: [
          '**Tài khoản › Mức dùng** hiện, trong **Còn lại bao nhiêu**, các khung của từng tài khoản (khung 5 giờ và các khung theo tuần) cùng thời điểm đặt lại, mức dùng thêm nếu có, lúc mức dùng được đọc và số phiên của tài khoản đó đang chạy.',
          '**Làm mới** đọc lại mức dùng của một tài khoản ngay lúc đó.',
          'Tài khoản token không tự đọc được gói của nó: mức dùng chỉ hiện sau khi có phiên chạy bằng tài khoản đó.',
          'Thanh đo mức dùng gói ở đầu trang **Phiên** mở trang này, còn thẻ **Thông tin** của một phiên hiện gói của tài khoản mà phiên đó dùng.',
          '**Giới hạn tốc độ mà các nhóm gặp** liệt kê các giới hạn tốc độ gần nhất mà Claude báo cho các phiên của nhóm; **Nhóm › Công suất** cho biết những phiên vai trò nào đang chờ một khung thời gian.',
        ],
        links: [{ label: 'Mở Mức dùng', path: '/accounts/usage' }],
      },
      {
        heading: 'Tần suất đọc mức dùng',
        steps: [
          'Mở **Tài khoản › Mức dùng** trong ứng dụng máy tính.',
          'Trong **Cách đọc mức dùng**, chọn tần suất ở **Đọc mức dùng mỗi**: **Mỗi 3 phút**, **Mỗi 5 phút (khuyên dùng)**, **Mỗi 10 phút** hoặc **Chỉ thủ công**.',
        ],
        points: [
          'Ứng dụng đọc theo nhịp đó khi có bảng điều khiển đang mở và một phiên đang hoạt động, và 30 phút một lần khi không có gì hoạt động.',
          'Với **Chỉ thủ công**, mức dùng được đọc khi bạn nhấn **Làm mới**, hoặc bởi một phiên do Mother Claude chạy khi có bảng điều khiển kết nối hay khi chạm giới hạn.',
          'Điện thoại hiện cài đặt này; bạn đổi nó trong ứng dụng máy tính.',
        ],
      },
      {
        heading: 'Đổi tên hoặc gỡ bỏ tài khoản',
        steps: [
          'Nhấn **Quản lý** ở dòng của tài khoản.',
          'Để đổi tên, sửa **Tên** rồi nhấn **Lưu**.',
          'Để gỡ bỏ, nhấn nút **Gỡ bỏ …** có tên tài khoản, rồi nhấn **Gỡ bỏ** (hoặc **Giữ lại**).',
        ],
        points: [
          'Gỡ bỏ chỉ làm ứng dụng quên tài khoản. Thư mục và thông tin đăng nhập Claude của nó vẫn còn; token thì bị xóa.',
          'Không thể gỡ bỏ một tài khoản khi các phiên do Mother Claude khởi chạy, hoặc API, vẫn đang dùng nó.',
          'Việc gỡ bỏ làm trong ứng dụng máy tính; điện thoại chỉ đổi tên được.',
        ],
      },
      {
        heading: 'Trên điện thoại',
        points: [
          'Điện thoại có thể đổi tên tài khoản, đặt mặc định, cho phép hoặc thu hồi quyền sao chép, di chuyển dữ liệu, xem mỗi gói còn lại bao nhiêu, chọn tài khoản cho phiên mới, và tiếp tục hoặc chuyển tài khoản cho một phiên.',
          'Thêm tài khoản, đăng nhập, token, gỡ bỏ, plugin và tần suất đọc mức dùng thì làm trong ứng dụng máy tính.',
        ],
      },
    ],
  },
  'move-data': {
    id: 'move-data',
    title: 'Di chuyển, sao chép và xóa',
    icon: 'move',
    group: 'data',
    summary: 'Di chuyển hoặc sao chép cuộc trò chuyện, dự án và tùy chỉnh; xóa chúng; hoàn tác.',
    intro:
      'Di chuyển hoặc sao chép bao nhiêu cuộc trò chuyện cũng được, cả dự án, hoặc mọi thứ của một tài khoản: giữa các tài khoản, và vào hoặc ra khỏi một kho lưu trữ. Mỗi lần di chuyển đều được kiểm tra trước và có thể hoàn tác trong bảy ngày.',
    keywords: [
      'chuyển',
      'di chuyển',
      'sao chép',
      'xóa',
      'hoàn tác',
      'lịch sử',
      'tùy chỉnh',
      'kỹ năng',
      'tệp',
      'thư mục',
      'đã xóa gần đây',
    ],
    sections: [
      {
        heading: 'Di chuyển hoặc sao chép',
        intro:
          'Bắt đầu từ **Chuyển hoặc sao chép dữ liệu…** ở dòng của một tài khoản trên trang **Tài khoản**, từ **Di chuyển hoặc sao chép…** trong menu **⋮** của một phiên, hoặc từ **Chọn** trên danh sách Phiên.',
        steps: [
          'Chọn **Từ** và **Đến**.',
          'Ở **Nội dung**, chọn **Tất cả**, **Dự án**, **Cuộc trò chuyện** hoặc **Tùy chỉnh**, rồi đánh dấu những gì cần lấy.',
          'Ở **Cách thức**, chọn **Sao chép** hoặc **Di chuyển**.',
          'Nhấn **Kiểm tra việc nó sẽ làm**.',
          'Đọc **Những gì được chuyển** và **Đã bỏ qua**.',
          'Nhấn nút ghi rõ thao tác, ví dụ **Di chuyển 3 cuộc trò chuyện**.',
        ],
        tips: [
          '**Đồng thời tin cậy thư mục của chúng cho** tài khoản đích sẽ chuyển cả quyền tin cậy thư mục của Claude Code sang.',
        ],
      },
      {
        heading: 'Sao chép tùy chỉnh',
        points: [
          'Ở **Nội dung › Tùy chỉnh**, đánh dấu bất kỳ mục nào trong **Bộ nhớ** (CLAUDE.md), **Cài đặt**, **Kỹ năng**, **Tác tử**, **Lệnh**, **Kiểu đầu ra**, **Phím tắt**, **Máy chủ MCP** và **Lịch sử lời nhắc**.',
          'Tùy chỉnh chỉ được sao chép, không bao giờ bị di chuyển, và chỉ giữa các tài khoản có thư mục khác nhau. Plugin không bao giờ được sao chép.',
          '**Cài đặt**, **Máy chủ MCP** và quyền tin cậy thư mục cần ứng dụng máy tính, hoặc một thiết bị được phép làm thao tác nguy hiểm.',
        ],
      },
      {
        heading: 'Xóa cuộc trò chuyện hoặc dự án',
        steps: [
          'Chọn **Xóa…** trong menu **⋮** của một phiên, trong menu của một dự án, hoặc nhấn **Chọn** rồi **Xóa N…**.',
          'Kiểm tra **Những gì được chuyển** và **Đã bỏ qua**.',
          'Giữ dấu chọn ở **Giữ bản sao trong 7 ngày để có thể hoàn tác**.',
          'Nhấn nút ghi rõ thao tác xóa, ví dụ **Xóa 2 cuộc trò chuyện**.',
        ],
        notes: [
          'Việc xóa luôn cần ứng dụng máy tính, hoặc một thiết bị được phép làm thao tác nguy hiểm. Cuộc trò chuyện đang chạy sẽ bị bỏ qua.',
        ],
      },
      {
        heading: 'Thay đổi gần đây',
        intro:
          '**Tài khoản › Thay đổi gần đây** liệt kê mọi thay đổi với dữ liệu của bạn, mới nhất trước: các lần di chuyển và sao chép giữa các tài khoản, sao lưu, khôi phục, kéo về và xóa, mỗi mục kèm kết quả của nó.',
        points: [
          'Các nút lọc chỉ hiện một loại: **Tất cả**, **Chuyển và sao chép**, **Bản sao lưu** hoặc **Đã xóa**.',
          'Danh sách tự cập nhật khi một công việc bắt đầu hoặc kết thúc, từ bất kỳ thiết bị nào, và công việc đang chạy cho biết nó đã làm được đến đâu.',
          '**Chuyển và sao chép** trên trang **Tài khoản**, **Gần đây** trên trang **Bản sao lưu** và **Đã xóa gần đây** trên **Phiên › Dự án** liệt kê ba mục mới nhất; nút **Xem tất cả** của chúng mở danh sách này ở đúng loại đó.',
          'Các phiên đã chuyển giữa các máy tính của bạn nằm ở **Máy tính › Hoạt động**.',
        ],
        links: [{ label: 'Mở Thay đổi gần đây', path: '/accounts/changes' }],
      },
      {
        heading: 'Hoàn tác',
        steps: [
          'Tìm công việc đó trên **Tài khoản › Thay đổi gần đây**, hoặc trong danh sách ngắn ở nơi bạn đã bắt đầu nó.',
          'Nhấn **Hoàn tác**.',
          'Nhấn **Chạm lần nữa để hoàn tác**.',
        ],
        points: [
          'Bạn có thể hoàn tác trong bảy ngày; công việc ghi rõ hạn đến khi nào.',
          'Hoàn tác bị từ chối nếu một bản sao đã được tiếp tục sau đó: cuộc trò chuyện ấy đã đi tiếp.',
        ],
      },
      {
        heading: 'Tìm tệp của một cuộc trò chuyện',
        steps: [
          'Mở phiên và chọn thẻ **Thông tin**.',
          'Cạnh **Bản ghi**, nhấn **Hiện trong Finder** hoặc **Hiện trong File Explorer** (ứng dụng máy tính), hoặc **Sao chép đường dẫn**.',
        ],
      },
      {
        heading: 'Những gì bị từ chối, và vì sao',
        points: [
          'Cuộc trò chuyện đang chạy sẽ bị bỏ qua: hãy dừng nó trước.',
          'Khi Claude Code đang chạy trong thư mục của tài khoản đích, các tệp dùng chung sẽ không bị thay đổi. Hãy thoát các phiên đó rồi thử lại; lần sao chép chỉ thêm tệp mới thì vẫn chạy.',
          'Giữa tài khoản Mặc định và một tài khoản token thì không có gì để sao chép: chúng dùng chung một thư mục, nên **Di chuyển** chỉ đổi tài khoản sở hữu.',
          'Mỗi lúc chỉ chạy một thao tác di chuyển, sao chép, sao lưu hoặc khôi phục.',
        ],
      },
    ],
  },
  backup: {
    id: 'backup',
    title: 'Bản sao lưu',
    icon: 'cloud',
    group: 'data',
    summary:
      'Bản sao lưu được mã hóa trong thư mục bạn chọn, và cuộc trò chuyện của bạn trên máy tính thứ hai.',
    intro:
      'Mother Claude giữ bản sao mã hóa các cuộc trò chuyện của từng tài khoản trong một thư mục bạn chọn (iCloud Drive, Google Drive, OneDrive, một ổ đĩa), trên trang **Bản sao lưu**. Chỉ cụm mật khẩu của bạn mới mở được bản sao này. Chính thư mục đó đưa cuộc trò chuyện sang máy tính khác: sao lưu trên máy này, kéo về trên máy kia. Không có gì tự diễn ra: bạn tự sao lưu và kéo về, hoặc đặt lịch. Việc giữ mã nguồn của một dự án giống nhau trên hai máy tính nằm ở **Máy tính › Đồng bộ mã nguồn**.',
    keywords: [
      'sao lưu',
      'khôi phục',
      'kéo về',
      'đồng bộ',
      'cụm mật khẩu',
      'kho lưu trữ',
      'lưu trữ',
      'cặp thư mục',
      'máy tính thứ hai',
      'kéo thả',
      'cuộc trò chuyện',
      'mã hóa',
      'lịch sao lưu',
    ],
    sections: [
      {
        heading: 'Thiết lập',
        intro:
          'Cho đến khi có bản sao lưu đầu tiên, trang sẽ mở với **Thiết lập sao lưu trong ba bước**.',
        steps: [
          'Mở **Bản sao lưu**.',
          'Bước 1: nhấn một trong các thư mục gợi ý (iCloud Drive, Google Drive…), hoặc nhấn **Nhập thư mục…**, nhập đường dẫn đầy đủ vào **Thư mục sao lưu** rồi nhấn **Lưu**.',
          'Bước 2: nhấn **Đặt cụm mật khẩu…** và làm theo hộp thoại (xem bên dưới).',
          'Bước 3: nhấn **Sao lưu ngay**.',
        ],
        tips: [
          'iCloud Drive: nhấp chuột phải vào thư mục và chọn **Keep Downloaded** (luôn giữ bản tải về), để kéo về và khôi phục không bao giờ phải chờ. OneDrive: **Always keep on this device** (luôn giữ trên thiết bị này). Google Drive: dùng một thư mục trong My Drive (Drive của tôi).',
          'Về sau, thư mục, cụm mật khẩu và lịch nằm ở mục **Cài đặt** trên cùng trang này. Để ngừng sao lưu, hãy xóa trống ô thư mục rồi lưu.',
        ],
        links: [{ label: 'Mở Bản sao lưu', path: '/backups' }],
      },
      {
        heading: 'Tổng quan bản sao lưu',
        intro: '**Bản sao lưu của bạn** hiện mỗi tài khoản trên một hàng, từ trái sang phải:',
        terms: [
          {
            term: 'Máy Mac này (hoặc Máy PC này)',
            text: 'Tài khoản trên máy tính này, loại tài khoản và số phiên của nó đang chạy.',
          },
          {
            term: 'Đường nối ở giữa',
            text: 'Tình trạng sao lưu: **Đã sao lưu 2 giờ trước** màu xanh lá; **Chưa sao lưu**, hoặc bản sao lưu đã cũ quá hai lần chu kỳ đã đặt, màu vàng; sao lưu thất bại hoặc thiếu khóa, màu đỏ; **Đang xử lý…** khi đang sao lưu, kéo về hoặc khôi phục.',
          },
          {
            term: 'Bản sao lưu',
            text: 'Bản sao lưu của tài khoản, với **Sao lưu**, **Kéo về…**, **Khôi phục…**, **Sao lưu buộc…**, **Khôi phục cài đặt** và **Cuộc trò chuyện**.',
          },
          {
            term: 'Máy tính khác',
            text: 'Những máy tính khác cũng sao lưu vào đó, và thời điểm gần nhất chúng sao lưu.',
          },
        ],
        tips: [
          'Trên ứng dụng máy tính, bạn có thể kéo thả thay vì nhấn nút: một tài khoản vào phần bản sao lưu, một bản sao lưu vào một tài khoản, các cuộc trò chuyện vào một tài khoản. Thao tác kéo thả nào cũng có một nút làm điều tương tự.',
        ],
      },
      {
        heading: 'Đặt cụm mật khẩu',
        where: 'desktop',
        steps: [
          'Ở **Cụm mật khẩu**, nhấn **Đặt cụm mật khẩu…**.',
          'Nhấn **Tạo cụm mật khẩu**.',
          'Ghi lại cụm mật khẩu: nó chỉ hiện một lần.',
          'Đánh dấu **Tôi đã ghi lại**.',
          'Nhấn **Xong**.',
        ],
        tips: [
          '**Dùng cụm của tôi…** để dùng cụm của riêng bạn, dài ít nhất 12 ký tự: nhập vào **Cụm mật khẩu** và **Nhập lại**, rồi nhấn **Lưu vào Chuỗi khóa**.',
          '**Đổi…** để đặt cụm mới sau này.',
        ],
        notes: [
          'Không có cụm mật khẩu thì không ai đọc được bản sao lưu — kể cả bạn. Mother Claude không thể lấy lại cụm mật khẩu.',
        ],
      },
      {
        heading: 'Sao lưu',
        steps: [
          'Ở **Cài đặt › Lịch**, chọn tần suất trong **Sao lưu**: từ **Chỉ thủ công** đến **Mỗi 24 giờ**.',
          'Để sao lưu ngay, nhấn **Sao lưu ngay** cạnh **Bản sao lưu của bạn** (mọi tài khoản), hoặc **Sao lưu** trên bản sao lưu của một tài khoản.',
          'Hoặc, trên ứng dụng máy tính, kéo thả một tài khoản từ **Máy Mac này** (hoặc **Máy PC này**) vào phần bản sao lưu: tài khoản đó được sao lưu ngay.',
          'Xem kết quả ở **Gần đây** (**Xem tất cả** liệt kê mọi lần sao lưu, khôi phục và kéo về).',
        ],
        points: [
          'Bản sao lưu theo lịch sẽ chờ đến khi không phiên nào của tài khoản đó hoạt động trong mười phút. Phiên mà Claude đang làm việc sẽ bị bỏ qua cho đến khi rảnh; phiên đang rảnh vẫn được sao lưu dù vẫn còn mở.',
          'Khi một lần sao lưu kết thúc, một thông báo cho biết kết quả, và lý do nếu có phần bị bỏ qua.',
          '**Kéo về khi khởi động** sẽ kéo về lúc ứng dụng khởi động, khi không có phiên nào đang chạy.',
          'Thông tin đăng nhập và token không bao giờ được sao lưu: mỗi máy tính tự đăng nhập riêng.',
        ],
      },
      {
        heading: 'Kéo thả để mang cuộc trò chuyện về',
        where: 'desktop',
        steps: [
          'Trên bản sao lưu của một tài khoản, nhấn **Cuộc trò chuyện**. Danh sách cho thấy bản sao lưu có những gì, theo từng dự án.',
          'Xem từng cuộc trò chuyện so với tài khoản ở đây ra sao: **Chưa có trên máy tính này**, **Mới hơn trong bản sao lưu**, **Thay đổi ở cả hai** hoặc **Đã cập nhật**. **Có thể đưa về** hiện những cuộc sẽ thay đổi điều gì đó; **Tất cả** hiện mọi thứ.',
          'Đánh dấu những cuộc bạn muốn, hoặc không đánh dấu gì nếu chỉ kéo thả một cuộc.',
          'Kéo thả chúng vào đúng tài khoản của chúng (hàng của tài khoản, hoặc nhãn của nó phía trên danh sách): chúng về đây ngay.',
          'Hoặc kéo thả chúng vào một tài khoản khác: **Khôi phục…** sẽ mở cho tài khoản đó, với các cuộc trò chuyện ấy đã được đánh dấu.',
        ],
        points: [
          'Mang về như vậy là kéo về riêng những cuộc đó: bản mới hơn thay bản ở đây (bản cũ được giữ để hoàn tác), và khi xung đột thì cả hai bên đều được giữ.',
          'Nếu Claude Code sẽ lại xóa một cuộc trò chuyện ở lần dọn dẹp tới, **Kéo về…** sẽ mở ra trước để báo cho bạn biết.',
          'Không kéo thả: đánh dấu chúng rồi nhấn nhãn tài khoản phía trên danh sách.',
        ],
      },
      {
        heading: 'Khôi phục một bản sao lưu',
        steps: [
          'Trên bản sao lưu của một tài khoản, nhấn **Khôi phục…** (hoặc kéo thả bản sao lưu vào một tài khoản khác để khôi phục vào tài khoản đó).',
          'Chọn **Bản sao lưu của** và **Khôi phục vào**.',
          'Đánh dấu các phiên cần khôi phục. Những phiên chưa có ở đây được đánh dấu sẵn; mỗi phiên ghi rõ nó so với bản ở đây ra sao.',
          'Nhấn **Khôi phục N phiên**.',
        ],
        points: [
          'Phiên cũ sẽ có cảnh báo trước: Claude Code xóa các cuộc trò chuyện cũ hơn chu kỳ dọn dẹp của nó. Nhấn **Vẫn khôi phục** để tiếp tục.',
          'Để bản sao lưu được ưu tiên hơn các bản mà tài khoản đang có, đánh dấu **Buộc: thay các bản mà … đang có**. Việc này cần ứng dụng máy tính, hoặc một thiết bị được phép làm thao tác nguy hiểm.',
        ],
      },
      {
        heading: 'Kéo về cuộc trò chuyện của máy tính khác',
        steps: [
          'Nhấn **Kéo về…** (cạnh **Bản sao lưu của bạn** cho mọi tài khoản, hoặc trên bản sao lưu của một tài khoản), hoặc kéo thả một bản sao lưu vào đúng tài khoản của nó.',
          'Kiểm tra việc sẽ làm với từng phiên, ví dụ **Mới hơn trong bản sao lưu — thay bản ở đây, bản cũ được giữ để hoàn tác**, **Đã đổi ở cả hai nơi — giữ cả hai bản song song** hoặc **Đang chạy ở đây — bỏ qua**.',
          'Nhấn **Kéo về N phiên**.',
        ],
        points: [
          'Phiên bị thay đổi trên cả hai máy tính sẽ được giữ thành hai bản, không bao giờ bị gộp.',
          'Để bản sao lưu được ưu tiên, đánh dấu **Buộc: bản trong bản sao lưu thay cho …**. **Sao lưu buộc…** trên một tài khoản làm điều ngược lại. Cả hai đều cần ứng dụng máy tính, hoặc một thiết bị được phép làm thao tác nguy hiểm.',
        ],
      },
      {
        heading: 'Hoàn tác một lần kéo về hoặc khôi phục',
        steps: [
          'Ở **Gần đây**, tìm công việc đó: **Xem tất cả** mở các công việc cũ hơn trên **Tài khoản › Thay đổi gần đây**.',
          'Nhấn **Hoàn tác**.',
          'Nhấn **Chạm lần nữa để hoàn tác**.',
        ],
        points: [
          'Bạn có thể hoàn tác trong bảy ngày. Bản thân một lần sao lưu thì không có gì để hoàn tác.',
        ],
        links: [{ label: 'Mở Thay đổi gần đây', path: '/accounts/changes' }],
      },
      {
        heading: 'Kho lưu trữ và bản lưu trữ',
        intro:
          'Ngoài thư mục sao lưu, bạn có thể thêm kho lưu trữ: các thư mục để lưu trữ cuộc trò chuyện và để chứa những bộ cuộc trò chuyện bạn chia sẻ.',
        steps: [
          'Ở **Kho lưu trữ**, nhấn **Thêm kho lưu trữ…**.',
          'Chọn một thư mục ở **Gợi ý:** hoặc nhập một thư mục vào **Thư mục**, thêm **Nhãn** nếu muốn, rồi nhấn **Thêm kho lưu trữ**.',
          'Trên một bộ, nhấn **Chuyển vào đây…** (hoặc **Bộ mới…** trên kho lưu trữ).',
          'Ở **Nội dung**, chọn **Cuộc trò chuyện** và đánh dấu những cuộc cần lấy.',
          'Ở **Cách thức**, chọn **Sao chép** để vẫn giữ chúng ở đây, hoặc **Di chuyển** để lưu trữ chúng.',
          'Nhấn **Kiểm tra việc nó sẽ làm**, rồi nhấn nút ghi rõ thao tác, ví dụ **Lưu trữ 3 cuộc trò chuyện**.',
        ],
        points: [
          '**Đưa về…** trên một bộ sẽ khôi phục các cuộc trò chuyện của bộ đó theo cùng cách.',
          'Khi tài khoản đã có một cuộc trò chuyện nhưng bản trong bộ lại khác (ví dụ bạn đã tiếp tục nó trên máy tính khác), nhấn **Thay bằng bản trong bộ**: bản của tài khoản được giữ để hoàn tác. Cuộc trò chuyện đang mở ở đây không bao giờ bị thay.',
          'Trên ứng dụng máy tính, kéo thả một tài khoản vào một bộ của kho lưu trữ để mở **Chuyển vào đây…** từ tài khoản đó, hoặc kéo thả một bộ vào một tài khoản để mở **Đưa về…** vào tài khoản đó.',
          'Gỡ bỏ một kho lưu trữ vẫn để nguyên các tệp của nó.',
        ],
      },
      {
        heading: 'Cặp thư mục',
        intro:
          'Một cặp nối một thư mục ở đây với thư mục chứa cùng các dự án trên máy tính khác, để phiên được kéo về nằm đúng dự án tương ứng ở đây (và ngược lại).',
        steps: [
          'Ở **Cặp thư mục**, nhập thư mục trên máy tính này vào **Thư mục trên …**.',
          'Chọn **Máy tính khác**.',
          'Điền **Thư mục trên máy đó**.',
          'Nhấn **Thêm cặp**.',
        ],
        tips: [
          'Máy tính khác chỉ hiện ở đây sau khi nó đã sao lưu vào hoặc kéo về từ các bản sao lưu này; **Xem lại** sẽ kiểm tra lại. Báo cáo của một lần kéo về có **Ghép cặp thư mục…** với các ô đã điền sẵn.',
        ],
      },
      {
        heading: 'Tiếp tục cuộc trò chuyện từ máy tính khác',
        intro:
          'Khi bạn tiếp tục một cuộc trò chuyện do máy tính khác bắt đầu mà chưa có cặp nào cho biết thư mục của nó ở đây, Mother Claude sẽ hỏi: **Chọn thư mục của nó trên máy PC này** (hoặc **trên máy Mac này**).',
        steps: [
          'Xem cuộc trò chuyện đã làm việc ở thư mục nào trên máy tính kia.',
          'Chọn một trong các thư mục ở đây trông giống nó (thư mục khả dĩ nhất đã được chọn sẵn), hoặc chọn **Thư mục khác** và nhập đường dẫn đầy đủ.',
          'Giữ dấu chọn ở ô **Ghép nối** để ghi nhớ sự tương ứng, hoặc bỏ dấu chọn nếu chỉ dùng cho cuộc trò chuyện này.',
          'Nhấn **Tiếp tục ở đây**.',
        ],
        points: [
          'Cuộc trò chuyện chạy trong thư mục bạn chọn; nó không bị di chuyển.',
          'Khi đánh dấu **Ghép nối**, hai thư mục được ghép cặp (nếu hai dự án cùng tên thì ghép thư mục cha của chúng), để các cuộc trò chuyện khác của máy tính kia ở đó tiếp tục ở đây mà không cần hỏi.',
          '**Hủy** không làm gì cả. Tin nhắn bạn đang gửi sẽ chưa được gửi, kèm nút **Thử lại**.',
          'Hộp thoại này hiện khi bạn nhấn **Tiếp tục ở đây**, gửi tin nhắn vào một cuộc trò chuyện đã kết thúc hoặc chọn **Tiếp tục trên tài khoản khác…**, và cả khi thư mục không còn trên máy tính này.',
          'Từ điện thoại, bạn chỉ chọn được thư mục mà Claude Code trên máy tính của bạn đã tin cậy.',
        ],
      },
      {
        heading: 'Máy Mac và Windows, từng bước',
        steps: [
          'Trên máy tính thứ nhất: chọn thư mục sao lưu, tạo cụm mật khẩu và nhấn **Sao lưu ngay**.',
          'Trên máy tính thứ hai: chọn cùng thư mục đó, theo đường dẫn mà máy này truy cập được.',
          'Trên máy tính thứ hai: nhấn **Đặt cụm mật khẩu…**, rồi **Dùng cụm của tôi…**, nhập đúng cụm mật khẩu đó và nhấn **Lưu vào Chuỗi khóa**.',
          'Tùy chọn: ở **Cặp thư mục**, ghép cặp thư mục dự án của bạn với thư mục tương ứng trên máy tính kia.',
          'Nhấn **Kéo về…**, rồi **Kéo về N phiên**.',
          'Mở một cuộc trò chuyện và nhấn **Tiếp tục ở đây**. Chọn thư mục của nó nếu được hỏi.',
          'Để mang về máy kia: nhấn **Sao lưu ngay** trên máy tính này, rồi **Kéo về…** trên máy kia.',
        ],
        tips: [
          'Hãy đặt cùng một tên cho mỗi tài khoản trên cả hai máy tính: bản sao lưu được ghép theo tên tài khoản.',
        ],
      },
      {
        heading: 'Từ điện thoại',
        points: [
          'Điện thoại đã ghép nối làm được mọi việc ở đây, trừ đặt hoặc đổi cụm mật khẩu. Điện thoại dùng các nút: kéo thả chỉ có trên ứng dụng máy tính.',
          'Thao tác buộc và quyền tin cậy thư mục cần một thiết bị được phép làm thao tác nguy hiểm (**Điện thoại & thiết bị** trong ứng dụng máy tính).',
        ],
      },
    ],
  },

  // ─────────────────── The browser (Settings and connections), then Teams
  browser: {
    id: 'browser',
    title: 'Tiện ích trình duyệt',
    icon: 'globe',
    group: 'settings',
    summary:
      'Cho phiên đọc và thao tác trong chính Chrome, Edge hoặc Brave của bạn, khi bạn cho phép.',
    intro:
      'Tiện ích Mother Claude cho các phiên Claude Code đọc và thao tác ngay trong trình duyệt của bạn, với các tài khoản bạn đang đăng nhập. Mỗi phiên chỉ làm việc trong nhóm thẻ có màu của riêng nó, mọi trang đều bắt đầu ở chế độ **Tắt**, và các bước rủi ro sẽ chờ bạn trả lời. Không có gì rời khỏi máy tính của bạn. Tiện ích chỉ có tiếng Anh, nên các nhãn của nó dưới đây được giữ nguyên.',
    keywords: [
      'tiện ích',
      'tiện ích mở rộng',
      'trình duyệt',
      'thẻ',
      'nhóm thẻ',
      'trang web',
      'chế độ trang',
      'bảng bên',
      'ghép nối',
      'tải lên',
    ],
    sections: [
      {
        heading: 'Cài tiện ích',
        steps: [
          'Trên trang **Releases** của dự án, ở mục **Assets**, tải `mother-claude-chrome-<version>.zip` về.',
          'Giải nén vào một thư mục bạn sẽ giữ lâu dài: trình duyệt nạp tiện ích từ đó mỗi lần khởi động.',
          'Mở `chrome://extensions` (`edge://extensions` trong Edge, `brave://extensions` trong Brave).',
          'Bật **Developer mode** (chế độ dành cho nhà phát triển).',
          'Nhấn **Load unpacked** (tải tiện ích đã giải nén) và chọn thư mục vừa giải nén (thư mục chứa `manifest.json`).',
          'Ghim **Mother Claude** từ menu hình mảnh ghép.',
        ],
        points: [
          'Tiện ích chạy trên Chrome, Edge, Brave và Chromium 116 trở lên, không chạy trên Firefox hay Safari. Mỗi trình duyệt ghép nối riêng.',
          'Điện thoại không chạy được tiện ích này.',
        ],
      },
      {
        heading: 'Ghép nối trình duyệt',
        steps: [
          'Trong Mother Claude, mở trang **Trình duyệt**.',
          'Nhấn **Ghép nối trình duyệt**. Một mã sẽ hiện ra; mã chỉ dùng được một lần, trong 5 phút.',
          'Trong trình duyệt, bấm nút Mother Claude: bảng bên mở ra ở **Pair this browser**.',
          'Nhập mã vào **Pairing code** và kiểm tra **Name of this browser**.',
          'Nhấn **Pair**.',
          'Khi bảng bên báo **Connected**, nhấn **Xong** trong Mother Claude.',
        ],
        notes: [
          'Việc ghép nối làm trong ứng dụng máy tính, hoặc trên điện thoại được phép làm thao tác nguy hiểm.',
        ],
        links: [{ label: 'Mở Trình duyệt', path: '/browser' }],
      },
      {
        heading: 'Chọn những gì phiên được làm trên một trang',
        intro: 'Mỗi trang có một chế độ: **Tắt** (mặc định), **Đọc** hoặc **Thao tác**.',
        steps: [
          'Ở **Chế độ trang**, nhấn **Tắt**, **Đọc** hoặc **Thao tác** trên **Mọi trang khác** hoặc trên dòng của một trang.',
          'Để thêm một trang, nhập địa chỉ của nó (ví dụ `https://github.com` hoặc `https://*.example.com`), chọn chế độ rồi nhấn **Thêm**.',
        ],
        points: [
          'Trong bảng bên, **This tab** đặt chế độ cho trang bạn đang mở. **Use the default for this site** xóa chế độ riêng đó.',
          'Mục riêng của một trang được ưu tiên hơn mẫu có ký tự đại diện, và `https://*.example.com` gồm các tên miền con nhưng không gồm chính `example.com`.',
          'Trang nhạy cảm (danh sách có sẵn gồm các trang thanh toán, ngân hàng và đăng nhập) luôn ở **Tắt** cho đến khi bạn đặt chế độ riêng cho trang đó, và luôn hỏi trước khi một phiên mở hoặc thao tác trên trang ấy.',
          '**Thao tác** trên **Mọi trang khác**, trên một mẫu có ký tự đại diện hoặc trên trang nhạy cảm cần ứng dụng máy tính, hoặc một thiết bị được phép làm thao tác nguy hiểm.',
        ],
      },
      {
        heading: 'Cấp công cụ trình duyệt cho phiên',
        points: [
          'Phiên dùng trình duyệt qua các công cụ của Mother Claude, vốn được bật sẵn (**Cài đặt › Công cụ cho Claude Code**).',
          'Phiên bắt đầu bên ngoài Mother Claude cần có thêm plugin Mother Claude (**Phiên bắt đầu ở nơi khác**).',
          'Phiên đã chạy từ trước cần `/reload-plugins` hoặc khởi động lại. Cho đến lúc đó, bảng bên hiện **No browser tools** cho phiên ấy.',
        ],
        links: [{ label: 'Xem Công cụ Claude Code', path: '/help/claude-code-tools' }],
      },
      {
        heading: 'Nhóm thẻ',
        points: [
          'Thao tác trình duyệt đầu tiên của một phiên sẽ mở một nhóm thẻ có màu, đặt tên theo phiên và dự án của nó. Phiên chỉ làm việc trong các thẻ của nhóm đó.',
          'Kéo một thẻ vào nhóm để giao nó cho phiên, hoặc kéo ra để lấy lại.',
          '**Sessions in this browser** trong bảng bên cho biết mỗi phiên đang làm gì.',
          'Một phiên vẫn kết nối với nhóm thẻ của nó cho đến khi bạn nhấn **Stop**, đóng nhóm, đóng trình duyệt hoặc thoát Mother Claude — dù bao lâu không có thao tác trình duyệt nào, và cả sau khi phiên kết thúc.',
        ],
      },
      {
        heading: 'Nhiều trang, mạng, cookie và dữ liệu lưu trữ',
        intro:
          'Các phiên có thể làm việc trên nhiều trang cùng lúc và xem những gì bạn thấy trong công cụ dành cho nhà phát triển.',
        points: [
          'Một phiên có thể giữ nhiều thẻ trong nhóm của nó, mỗi thẻ một trang, và làm việc song song trên chúng. Mỗi trang giữ chế độ riêng.',
          '**Mạng**: các yêu cầu một thẻ đã gửi (địa chỉ, trạng thái, kích thước, thời gian), một yêu cầu đầy đủ với header và nội dung, và các tin nhắn WebSocket. Việc ghi lại bắt đầu từ thao tác đầu tiên của phiên trong thẻ: tải lại trang để thấy các yêu cầu của nó từ đầu.',
          '**Cookie**: tên, hạn dùng và các cờ của những cookie được gửi tới trang. Giá trị của chúng, và việc đặt hay xóa một cookie, đều chờ bạn duyệt.',
          '**Dữ liệu lưu trữ**: local storage và session storage của trang, và các cơ sở dữ liệu IndexedDB của nó (chỉ đọc). Giá trị, và mọi thay đổi, đều chờ bạn duyệt.',
          'Các header đăng nhập (như `Authorization` và `Cookie`) luôn hiện là `«redacted»`.',
          'Yêu cầu mạng, cookie, ảnh chụp màn hình và console cần quyền **debugger** của tiện ích. Chrome hiện một thanh thông báo khi quyền này đang được dùng.',
        ],
      },
      {
        heading: 'Yêu cầu duyệt',
        points: [
          'Yêu cầu hiện trong Hộp thư, trên trang của phiên, ở **Đang chờ bạn** trên trang **Trình duyệt**, và trong bảng bên. Câu trả lời đầu tiên được tính; không trả lời trong 5 phút được coi là từ chối.',
          'Luôn hỏi khi: vào trang nhạy cảm, điền biểu mẫu có ô mật khẩu, thẻ thanh toán hoặc mã dùng một lần, chạy script, tải tệp lên, đọc hoặc thay đổi giá trị cookie và dữ liệu lưu trữ, vượt giới hạn mỗi phút, và mọi thao tác trong một phút sau khi bị dừng (Stop).',
          '**Luôn cho phép…** trên một yêu cầu duyệt sẽ lưu một quyền cho phép lâu dài. Các quyền này được liệt kê ở **Luôn cho phép**, nơi **Gỡ bỏ** sẽ rút lại một quyền.',
        ],
      },
      {
        heading: 'Dừng và tiếp tục',
        steps: ['Trong bảng bên, nhấn **Stop** trên phiên đó.'],
        points: [
          'Lệnh đang chạy của phiên sẽ thất bại, các yêu cầu duyệt của nó bị rút lại, và thao tác trình duyệt của nó bị từ chối trong 60 giây. Đóng nhóm thẻ của phiên cũng có tác dụng như vậy.',
          'Muốn gỡ sớm hơn, nhấn **Resume** trong bảng bên, hoặc **Tiếp tục** ở **Phiên đang dùng trình duyệt** trên trang **Trình duyệt** (nơi đó cũng có **Dừng**).',
          'Trong một phút đó, các phiên khác thao tác trong trình duyệt ấy cần được cho phép.',
        ],
      },
      {
        heading: 'Làm việc trên thẻ đang mở',
        intro: 'Đưa một phiên bạn đang có tới trang trước mặt bạn.',
        steps: [
          'Mở trang đó và bấm nút Mother Claude.',
          'Ở **This tab**, nhấn **Read** hoặc **Act**.',
          'Ở **Work here with a session**, chọn phiên (nhấn **Refresh** nếu phiên chưa có trong danh sách).',
          'Viết vào **What should it do here?**.',
          'Nhấn **Connect and send**.',
        ],
        points: [
          'Thẻ sẽ vào nhóm thẻ của phiên. Về sau, nhấn **Tell it what to do** trên ô của phiên để gửi thêm.',
          'Phiên nhận lời bạn viết và trang web (chỉ địa chỉ gốc), không bao giờ nhận tiêu đề hay đường dẫn của trang.',
          'Phiên chạy mà không hỏi xin quyền chỉ nhận chỉ dẫn từ ứng dụng máy tính.',
        ],
      },
      {
        heading: 'Những việc không bao giờ làm',
        points: [
          'Phiên không bao giờ nhập vào ô mật khẩu, thẻ thanh toán, mã dùng một lần hay các ô bí mật khác: phiên sẽ nhờ bạn tự nhập.',
          'Tệp tải lên chỉ lấy từ thư mục dự án của phiên hoặc thư mục bạn đã thêm ở **Thư mục các phiên được tải tệp lên từ đó**, trong **Tùy chọn** trên trang **Trình duyệt**, không bao giờ từ các thư mục của Claude Code, `~/.ssh` hay những tệp thường chứa bí mật.',
          '**Hoạt động** ghi lại công cụ, trang, đường dẫn, kết quả và thời gian, không bao giờ ghi nội dung đã nhập hay đã đọc.',
        ],
      },
      {
        heading: 'Gỡ bỏ trình duyệt, hoặc cập nhật tiện ích',
        points: [
          'Để hủy ghép nối, nhấn **Gỡ bỏ trình duyệt này** (biểu tượng thùng rác) cạnh trình duyệt đó trong danh sách **Trình duyệt**, rồi nhấn **Gỡ bỏ**. Việc này cần ứng dụng máy tính, hoặc một thiết bị được phép làm thao tác nguy hiểm.',
          'Để cập nhật, giải nén bản phát hành mới vào cùng thư mục đó và nhấn mũi tên tải lại trên ô của tiện ích trong `chrome://extensions`. Ghép nối vẫn được giữ.',
        ],
      },
    ],
  },
  teams: {
    id: 'teams',
    title: 'Nhóm phiên',
    icon: 'layers',
    group: 'teams',
    summary: 'Một trưởng nhóm và các phiên vai trò cùng làm theo bảng tác vụ trong một dự án.',
    intro:
      'Một nhóm áp dụng một mẫu cho một dự án: phiên trưởng nhóm viết tác vụ lên bảng của dự án, và các phiên vai trò thực hiện chúng, mỗi phiên trên các tệp của riêng mình. Mother Claude khởi chạy, đánh thức và dừng các phiên do nó chạy, tùy theo công suất cho phép. **Nhóm** có ba thẻ: các nhóm của bạn, các **Mẫu quy trình** để bắt đầu nhóm, và **Công suất**.',
    keywords: [
      'nhóm',
      'vai trò',
      'trưởng nhóm',
      'bảng tác vụ',
      'tác vụ',
      'mẫu',
      'quy trình',
      'nhánh',
      'luồng công việc',
      'chuyển giao',
      'sơ đồ',
    ],
    sections: [
      {
        heading: 'Bắt đầu một nhóm',
        steps: [
          'Mở **Nhóm** và nhấn **Bắt đầu một nhóm** (có trên mọi thẻ của Nhóm).',
          'Chọn **Dự án**.',
          'Chọn một **Mẫu**.',
          'Nếu có nhiều tài khoản, chọn **Tài khoản**.',
          'Tùy chọn: viết **Yêu cầu cho trưởng nhóm (không bắt buộc)**.',
          'Ở **Vai trò**, đổi **Mô hình**, **Mức nỗ lực**, **Chế độ quyền**, **Máy tính** hoặc **Bắt đầu cùng nhóm** của một vai trò nếu muốn.',
          'Nhấn **Bắt đầu nhóm**.',
        ],
        points: [
          'Các mẫu có sẵn: Software delivery (phát triển phần mềm), Content production (sản xuất nội dung), Research & analysis (nghiên cứu và phân tích), Browser operations (thao tác trình duyệt) và Documentation (tài liệu).',
          'Mỗi dự án một nhóm. Vai trò có worktree riêng cần dự án là một kho git.',
        ],
        tips: [
          'Các nhóm tác tử của chính Claude Code, trong các phiên tương tác của bạn, không phải nhóm của Mother Claude: những gì chúng báo nằm ở **Phiên › Tác tử › Đồng đội**.',
        ],
        links: [{ label: 'Mở Nhóm', path: '/teams' }],
      },
      {
        heading: 'Bảng tác vụ',
        points: [
          'Tác vụ đi qua các cột **Cần làm**, **Đã nhận**, **Đang làm**, **Xem xét**, **Xong**, **Thất bại** và **Đã hủy**. Trên điện thoại, ô chọn **Trạng thái** hiện từng cột một.',
          'Trưởng nhóm và các phiên vai trò tự thêm và cập nhật tác vụ. Bạn cũng có thể thêm.',
        ],
      },
      {
        heading: 'Thêm tác vụ',
        steps: [
          'Trong nhóm, nhấn **Tác vụ mới**.',
          'Điền **Tiêu đề**, **Mục tiêu** và **Tiêu chí nghiệm thu (mỗi dòng một tiêu chí)**.',
          'Chọn **Vai trò** và **Ưu tiên**.',
          'Tùy chọn: chọn **Loại** và điền **Tệp (mỗi dòng một tệp)** và **Ràng buộc (mỗi dòng một ràng buộc)**.',
          'Nhấn **Thêm tác vụ**.',
        ],
      },
      {
        heading: 'Xử lý một tác vụ',
        intro: 'Chạm vào một tác vụ để mở nó.',
        points: [
          'Đổi **Ưu tiên** hoặc **Vai trò** của tác vụ khi nó chưa kết thúc.',
          'Nhập **Ghi chú cho phiên** và nhấn **Thêm ghi chú**.',
          'Nhấn **Bỏ chặn** trên tác vụ bị chặn.',
          'Ở cột Xem xét, nhấn **Chấp nhận** hoặc **Trả lại**. Muốn nâng cấp tác vụ, hãy đánh dấu **Khi trả lại: dùng mô hình mạnh hơn hoặc nỗ lực cao hơn** trước.',
          '**Hủy tác vụ** để dừng nó; **Mở lại** đưa tác vụ đã kết thúc trở lại.',
        ],
      },
      {
        heading: 'Phiên vai trò',
        points: [
          'Vai trò có việc sẽ bắt đầu với tác vụ gấp nhất của nó, khi công suất cho phép; vai trò đang rảnh nhận tác vụ kế tiếp. Khi không còn việc, nó dừng sau số phút rảnh mà mẫu quy định, không bao giờ dừng giữa lượt. Cuộc trò chuyện của nó được giữ lại.',
          '**Dừng**, **Bắt đầu** và **Tiếp tục** trên ô của một vai trò chỉ tác động lên vai trò đó.',
          'Chạm vào phiên của vai trò để mở và nhắn tin cho nó như mọi phiên khác.',
          'Vai trò phải chờ sẽ hiện **Đang chờ** kèm lý do; **Nhóm › Công suất** liệt kê mọi vai trò đang chờ, và vì sao.',
        ],
        links: [{ label: 'Xem Công suất, định tuyến và ngân sách', path: '/help/capacity' }],
      },
      {
        heading: 'Tệp của từng vai trò',
        points: [
          'Mỗi vai trò chỉ được sửa các tệp mà mẫu giao cho nó (danh sách **sở hữu …**); vai trò chỉ đọc thì không sửa gì. Lần ghi bị từ chối sẽ hiện thông báo, và ô của vai trò đếm số **lần bị chặn**.',
          'Đây là hàng rào, không phải môi trường cách ly (sandbox): ghi qua script, công cụ build hay git sẽ không bị chặn.',
          'Vai trò dùng worktree làm việc trong `<project>/.claude/worktrees/<role>` trên nhánh riêng, được Mother Claude giữ lại. Trưởng nhóm sẽ gộp các nhánh.',
        ],
        tips: ['Thêm `.claude/worktrees/` vào `.gitignore` của bạn.'],
      },
      {
        heading: 'Một vai trò trên máy tính khác',
        intro:
          'Một vai trò có thể chạy trên một máy tính khác của bạn: trưởng nhóm lên kế hoạch trên máy Mac trong khi một vai trò Windows làm việc trên máy PC, trên cùng một bảng tác vụ.',
        steps: [
          'Trên máy tính sẽ chạy vai trò, cấp cho máy này quyền **Chạy** trong **Máy tính**.',
          'Trong mẫu, thêm `"machine": "DESKTOP-PC"` cho vai trò (tên một máy đã ghép đôi, hoặc `os:windows` cho một máy Windows bất kỳ), hoặc chọn **Máy tính** của nó khi bắt đầu nhóm.',
          'Bắt đầu nhóm. Vai trò khởi chạy trên máy đó khi có việc.',
        ],
        points: [
          'Ở đó nó làm việc trong một worktree riêng, với mã nguồn của dự án này. Bản sao của bạn ở đó không bị động đến.',
          'Các tác vụ của nó nằm trên bảng này. Khi xong một tác vụ, kết quả của nó quay về đây thành nhánh `mother-claude/<computer>/<team>/<role>`, được ghi trên tác vụ; trưởng nhóm xem xét rồi gộp nhánh đó.',
          'Ô của nó ghi **trên DESKTOP-PC**, kèm lý do nó phải chờ hoặc không khởi chạy được. **Nhóm › Công suất** của máy kia tính phiên này, không phải của máy này; máy này liệt kê nó riêng, trong mục **Trên các máy tính khác của bạn**, kèm quyết định gần nhất của máy kia.',
          'Nó không nhắn được cho trưởng nhóm bằng SendMessage: nó ghi lên tác vụ của mình, và Mother Claude báo cho trưởng nhóm.',
          'Khi phiên của nó đã dừng, tin nhắn bạn gửi cho nó từ máy nào cũng khởi chạy lại nó với các thiết lập của vai trò, rồi đến được với nó. Nếu máy kia chưa thể chạy nó lúc này, tin nhắn bị từ chối kèm lý do.',
          'Máy kia liệt kê nó trong **Máy tính › Hoạt động › Vai trò trong nhóm**, nơi **Dừng** và **Gỡ bỏ** tác động lên nó ở đó.',
        ],
        links: [{ label: 'Xem “Giao tác vụ cho máy tính khác”', path: '/help/machines-task' }],
      },
      {
        heading: 'Dùng phiên bắt đầu ở nơi khác làm một vai trò',
        steps: ['Trên một vai trò đang không chạy, mở **Giao cho phiên…**.', 'Chọn phiên.'],
        points: [
          'Việc này cần plugin Mother Claude (**Phiên bắt đầu ở nơi khác**), và phiên phải chạy trong dự án. Mother Claude báo cho phiên về các tác vụ của nó nhưng không bao giờ khởi chạy hay dừng nó.',
          '**Nhả vai trò** để trả lại vai trò; các tác vụ của nó quay về bảng.',
        ],
      },
      {
        heading: 'Dừng, tiếp tục hoặc gỡ bỏ nhóm',
        steps: [
          'Nhấn **Dừng nhóm** và xác nhận. Mọi phiên vai trò sẽ dừng; cuộc trò chuyện của chúng được giữ lại.',
          'Nhấn **Tiếp tục** để chạy lại nhóm.',
          'Để xóa một nhóm đã dừng, nhấn **Gỡ bỏ** và xác nhận. Bảng của nhóm vẫn ở lại với dự án.',
        ],
      },
      {
        heading: 'Mẫu',
        points: [
          'Thẻ **Mẫu quy trình** liệt kê các mẫu có sẵn và mẫu của bạn, mỗi mẫu với các vai trò, mô hình của chúng và luồng công việc: kết quả của một vai trò được chuyển tiếp cho ai.',
          '**Mẫu mới** để tự viết mẫu; mẫu có sẵn thì cố định, nên muốn đổi một mẫu, hãy nhấn **Xem** rồi **Nhân bản để sửa**.',
          'Mẫu của bạn có **Sửa**, **Nhân bản** và xóa. **Sửa** mở mẫu dưới dạng sơ đồ (**Trực quan**): kéo một vai trò để di chuyển, và kéo từ chấm ở cạnh phải của nó sang một vai trò khác để chuyển kết quả cho vai trò đó, hoặc sang **Gộp** để chọn ai gộp kết quả. **Thêm vai trò** và **Tự sắp xếp** nằm phía trên sơ đồ.',
          'Chọn một vai trò, một mũi tên hoặc **Gộp** để sửa trong bảng bên cạnh sơ đồ (bên dưới trên điện thoại); bảng này làm được mọi việc mà thao tác kéo làm. **JSON** hiện cùng mẫu đó dưới dạng văn bản; **Kiểm tra** đặt từng vấn đề lên vai trò hoặc mũi tên của nó và liệt kê ra, còn **Lưu** giữ lại mẫu.',
        ],
        links: [{ label: 'Mở Mẫu quy trình', path: '/teams/templates' }],
      },
    ],
  },
  capacity: {
    id: 'capacity',
    title: 'Công suất, định tuyến và ngân sách',
    icon: 'gauge',
    group: 'teams',
    summary:
      'Vì sao một phiên vai trò phải chờ, giới hạn của bạn, ngân sách, cách chọn mô hình và chi phí.',
    intro:
      '**Công suất**, một thẻ của **Nhóm**, cho biết có bao nhiêu phiên vai trò đang chạy và vì sao các phiên khác phải chờ, kèm bộ nhớ và mức dùng gói của từng tài khoản, trong năm mục: **Hiện tại**, **Ngân sách**, **Mô hình**, **Chi phí** và **Cài đặt**. Các con số tiền chỉ là ước tính.',
    keywords: [
      'giới hạn',
      'ngân sách',
      'chi phí',
      'định tuyến',
      'mô hình',
      'bộ phân loại',
      'báo cáo',
      'đang chờ',
      'bộ nhớ',
      'tiền',
    ],
    sections: [
      {
        heading: 'Vì sao một phiên vai trò phải chờ',
        intro:
          'Mục **Hiện tại** cho thấy các phiên vai trò đang chạy và đang chờ, mỗi phiên kèm lý do bằng lời dễ hiểu. Một vai trò phải chờ khi:',
        points: [
          'Đã chạm giới hạn số phiên vai trò của bạn (không tính trưởng nhóm), hoặc của dự án.',
          'Không đủ bộ nhớ trống.',
          'Khung 5 giờ của tài khoản gần cạn: từ 85 % chỉ trưởng nhóm được bắt đầu, từ 95 % thì không phiên nào.',
          'Claude đã từ chối một yêu cầu vì giới hạn tần suất: việc khởi chạy phải chờ đến lúc giới hạn được đặt lại.',
          'Ngân sách theo ngày của dự án đã dùng hết.',
        ],
        tips: [
          'Vai trò chờ quá 10 phút sẽ gửi thông báo đẩy.',
          'Các giới hạn tốc độ mà Claude đã báo, và gói của mỗi tài khoản còn lại bao nhiêu, cũng có trên **Tài khoản › Mức dùng**.',
        ],
        links: [
          { label: 'Mở Công suất', path: '/teams/capacity' },
          { label: 'Mở Mức dùng', path: '/accounts/usage' },
        ],
      },
      {
        heading: 'Đặt giới hạn',
        steps: [
          'Mở **Nhóm › Công suất › Cài đặt** và nhấn **Sửa**.',
          'Đặt **Giới hạn của bạn** và **Giới hạn mỗi dự án** (từ 1 đến 16).',
          'Cho riêng một thư mục, nhấn **Thêm dự án** ở **Giới hạn riêng cho dự án**.',
          'Nhấn **Lưu**.',
        ],
        points: [
          'Mặc định là tổng cộng 3 phiên vai trò và 4 phiên mỗi dự án, luôn chừa trống 2 GB bộ nhớ. Ngưỡng bộ nhớ và ngưỡng mức dùng gói cũng nằm ở đó.',
        ],
      },
      {
        heading: 'Ngân sách',
        steps: [
          'Mở **Nhóm › Công suất › Ngân sách** và nhấn **Sửa ngân sách**.',
          'Đặt **Mỗi dự án, theo ngày (USD)**, **Một phiên vai trò (USD)** và **Một tác vụ (USD)**. Để trống nghĩa là không có ngân sách.',
          'Đặt **Cảnh báo ở mức (%)** và cách nhận cảnh báo.',
          'Nhấn **Lưu ngân sách**.',
        ],
        points: [
          'Khi ngân sách theo ngày của một dự án đã hết, không phiên làm việc mới nào được bắt đầu ở đó cho đến ngày mai (trưởng nhóm thì vẫn được). Từ mức cảnh báo trở đi, việc lẽ ra dùng Opus ở dự án đó sẽ chạy bằng Sonnet.',
          'Ngân sách cho một phiên vai trò chỉ được áp dụng với tài khoản trả tiền theo token; với tài khoản dùng gói, nó chỉ được hiển thị chứ không áp dụng.',
        ],
      },
      {
        heading: 'Cách chọn mô hình',
        where: 'desktop',
        intro:
          'Mỗi loại tác vụ có mô hình và mức nỗ lực riêng: lập kế hoạch, kiến trúc và rà soát dùng Opus; viết mã, kiểm thử và sửa lỗi dùng Sonnet; tìm kiếm và tóm tắt dùng Haiku. Tác vụ liên tục không qua được kiểm tra sẽ được nâng lên một bậc.',
        steps: [
          'Mở **Nhóm › Công suất › Mô hình** và nhấn **Sửa chính sách**.',
          'Đổi **Loại tác vụ**, **Thang nâng bậc**, **Chốt ngân sách** hoặc **Mô hình dự phòng**.',
          'Nhấn **Lưu chính sách**.',
        ],
        points: [
          '**Đặt lại mặc định** đưa chính sách về như ban đầu.',
          'Điện thoại chỉ xem được chính sách; việc sửa làm trong ứng dụng máy tính.',
        ],
      },
      {
        heading: 'Bộ phân loại tác vụ',
        where: 'desktop',
        points: [
          'Ở **Bộ phân loại tác vụ**, **Phân loại bằng Claude Haiku (tốn token)** dùng một mô hình để phân loại tác vụ thay vì dựa vào từ khóa. Tùy chọn này mặc định tắt và cần một khóa API Anthropic.',
          '**Cách tài khoản trả phí** cho biết mỗi tài khoản dùng gói hay trả tiền theo token.',
        ],
      },
      {
        heading: 'Chi phí',
        points: [
          'Mục **Chi phí** hiện chi phí và kết quả của các tác vụ trong **7 ngày**, **30 ngày** hoặc **90 ngày**. **CSV** và **Mọi tác vụ** để tải chúng về.',
          'Với tài khoản dùng gói, chi phí là số tiền mà API lẽ ra sẽ tính.',
        ],
      },
      {
        heading: 'Trên điện thoại',
        points: [
          'Điện thoại xem được mọi thứ ở đây và có thể đổi giới hạn và ngân sách.',
          'Chính sách định tuyến, cách tài khoản thanh toán và bộ phân loại thì đổi trong ứng dụng máy tính.',
        ],
      },
    ],
  },

  // ───────────── Claude Code tools and the HTTP API (Settings and connections)
  'claude-code-tools': {
    id: 'claude-code-tools',
    title: 'Công cụ Claude Code',
    icon: 'zap',
    group: 'settings',
    summary:
      'Công cụ riêng của Mother Claude cho các phiên của bạn: trình duyệt, bảng tác vụ và nhiều thứ khác.',
    intro:
      'Mother Claude cấp cho mọi phiên Claude Code trên máy tính của bạn các công cụ riêng: trình duyệt, bảng tác vụ của nhóm, các nhóm, các máy tính khác của bạn (`machine_run` và các công cụ đi kèm), và chính Mother Claude. Phiên thấy chúng dưới tên `mcp__plugin_mother-claude_mc__<tool>`.',
    keywords: ['công cụ', 'máy chủ MCP', 'dịch vụ', 'bật công cụ', 'tắt công cụ'],
    sections: [
      {
        heading: 'Bật hoặc tắt',
        where: 'desktop',
        steps: [
          'Mở **Cài đặt** và, ở **Claude Code**, chọn **Công cụ cho Claude Code**.',
          'Ở **Công cụ cho Claude Code (MCP)**, bật hoặc tắt **Cấp công cụ của Mother Claude cho các phiên**.',
        ],
        points: [
          'Mặc định chúng được bật. Phiên bắt đầu từ lúc đó trở đi sẽ có chúng; phiên đang chạy thì có sau `/reload-plugins` hoặc khi khởi động lại.',
          'Khi tắt: phiên không có công cụ, và lệnh gọi từ các phiên đang chạy bị từ chối.',
        ],
        links: [{ label: 'Mở Công cụ cho Claude Code', path: '/settings/tools' }],
      },
      {
        heading: 'Phiên nào có công cụ',
        points: [
          'Phiên do Mother Claude khởi chạy tự có công cụ.',
          'Phiên trong terminal, VS Code và JetBrains có công cụ qua plugin Mother Claude: hãy cài plugin đó trong **Phiên bắt đầu ở nơi khác**.',
          'Cuộc trò chuyện bắt đầu qua HTTP API hiện chưa có công cụ.',
          'Trang này hiện số **Công cụ**, **Phiên đã kết nối (15 phút qua)**, việc **Phiên do Mother Claude khởi chạy** có công cụ hay không, plugin, và **Lệnh gọi gần đây**.',
          'Khi plugin chưa được cài, hoặc cũ hơn ứng dụng này, một dòng sẽ báo điều đó, kèm liên kết tới **Phiên bắt đầu ở nơi khác**.',
        ],
        links: [{ label: 'Mở Phiên bắt đầu ở nơi khác', path: '/elsewhere' }],
      },
      {
        heading: 'Yêu cầu duyệt',
        points: [
          'Ở chế độ quyền mặc định, Claude Code hỏi trước khi dùng một trong các công cụ này. Phiên do Mother Claude chạy hỏi bằng một thẻ yêu cầu duyệt; phiên bắt đầu ở nơi khác hỏi trong cửa sổ của chính nó.',
          'Mọi việc nguy hiểm mà một công cụ định làm đều trở thành thẻ yêu cầu duyệt trước.',
        ],
      },
      {
        heading: 'Những gì Claude Code chạy ngoài các phiên',
        intro:
          'Ngoài các phiên, Claude Code còn chạy một dịch vụ nền, các máy chủ MCP và công việc nền. **Máy tính › Máy Mac này › Chi tiết** (**Máy PC này** trên Windows) hiện những thứ đó, và liên kết ở cuối **Cài đặt › Công cụ cho Claude Code** dẫn tới đó.',
        terms: [
          {
            term: 'Dịch vụ nền của Claude Code',
            text: 'Daemon của nó: có kết nối được hay không, và trạng thái của nó.',
          },
          { term: 'Máy chủ MCP', text: 'Các máy chủ MCP đã cấu hình cho Claude Code, theo tên.' },
          { term: 'Công việc nền', text: 'Các công việc chạy bằng `claude --bg`.' },
        ],
        points: ['**Kiểm tra lại** sẽ đọc lại tất cả.'],
      },
    ],
  },
  api: {
    id: 'api',
    title: 'HTTP API',
    icon: 'code',
    group: 'settings',
    summary: 'Điều khiển Claude trên máy tính này từ script và công cụ, qua HTTP.',
    intro:
      'Mother Claude có thể chạy một HTTP API cục bộ để điều khiển Claude trên máy tính này bằng tài khoản của bạn: cuộc trò chuyện, lượt, sự kiện trực tiếp và yêu cầu duyệt công cụ. Cuộc trò chuyện bắt đầu qua API là một phiên bình thường trên bảng điều khiển, và yêu cầu duyệt của nó hiện dưới dạng thẻ.',
    keywords: ['cầu nối', 'tập lệnh', 'cổng', 'máy chủ API', 'tự động hóa', 'lập trình'],
    sections: [
      {
        heading: 'Khởi động API',
        steps: [
          'Mở **HTTP API**.',
          'Ở **Mặc định**, điền **Thư mục làm việc** (để trống là thư mục chính của bạn).',
          'Chọn **Mô hình**, **Mức nỗ lực**, **Suy nghĩ** và **Quyền**, hoặc để **Mặc định** để dùng cài đặt Claude của riêng bạn.',
          'Ở **Cuộc trò chuyện**, dùng **Tin nhắn gửi vào** để chọn cuộc trò chuyện nhận tin nhắn.',
          'Nhấn **Khởi động API**.',
        ],
        points: [
          'Mỗi lần mở ứng dụng, API đều tắt, và không có gì lắng nghe cho đến khi bạn khởi động nó.',
          'Những gì bạn chọn ở đây chỉ là mặc định: mỗi yêu cầu đều có thể ghi đè.',
          'Để áp dụng thay đổi về sau, nhấn **Khởi động lại với cài đặt này**. **Đổi cuộc trò chuyện** chỉ đổi cuộc trò chuyện.',
          '**Dừng** sẽ đóng cổng; các cuộc trò chuyện đang chạy vẫn tiếp tục.',
        ],
        links: [{ label: 'Mở HTTP API', path: '/http-api' }],
      },
      {
        heading: 'Dùng thử',
        points: [
          'Khi API đang chạy, **Tài liệu API** mở trang tham chiếu tương tác, còn **Console** mở một trang để bạn trò chuyện, xem luồng sự kiện và cho phép các lệnh gọi công cụ.',
          'Màn hình hiện sẵn một lệnh `curl` theo cài đặt của bạn.',
          'Các liên kết này chỉ dùng được trên chính máy tính đó.',
        ],
      },
      {
        heading: 'Token',
        points: [
          'Chương trình chạy trên máy tính này không cần token.',
          'Yêu cầu đến qua đường hầm cần token, và khi có đường hầm trỏ vào cổng API thì mọi yêu cầu đều cần.',
          'Dòng **Xác thực** trên trang **HTTP API** cho biết lúc này cần gì.',
        ],
      },
      {
        heading: 'Mở cho đường hầm',
        where: 'desktop',
        points: [
          'Ở **Truy cập**, **Mở cho đường hầm** nhận lệnh gọi không cần token từ bất kỳ ai truy cập được cổng, kể cả qua đường hầm. Khởi động lại API để áp dụng.',
          'Khi đó, bất kỳ ai có địa chỉ đường hầm đều có thể bắt đầu cuộc trò chuyện và cho phép các lệnh gọi công cụ thông thường. Hãy để tắt nếu không cần; tùy chọn này tự tắt lại sau mỗi lần khởi động lại ứng dụng.',
        ],
      },
      {
        heading: 'Mỗi tài khoản một máy chủ API',
        steps: [
          'Nhấn **Thêm máy chủ API**.',
          'Điền **Tên** và, nếu có nhiều tài khoản, chọn **Tài khoản**.',
          'Để trống **Cổng** để dùng cổng trống kế tiếp.',
          'Nhấn **Thêm máy chủ API**.',
          'Kiểm tra các mặc định của máy chủ, rồi khởi động nó, hoặc nhấn **Lưu mà không khởi động**.',
        ],
        points: [
          '**Chỉ tài khoản này** khiến máy chủ chỉ chạy bằng một tài khoản: qua máy chủ đó, cuộc trò chuyện của các tài khoản khác coi như không tồn tại.',
          '**Quản lý** để đổi tên máy chủ, đổi cổng hoặc gỡ bỏ nó. Gỡ bỏ sẽ dừng máy chủ; các cuộc trò chuyện của nó vẫn được giữ.',
        ],
      },
    ],
  },

  // ─────────────────────────────────────────────────── Troubleshooting
  troubleshooting: {
    id: 'troubleshooting',
    title: 'Khắc phục sự cố',
    icon: 'question',
    group: 'troubleshooting',
    summary: 'Ý nghĩa của các thông báo, và cách xử lý.',
    intro:
      'Tìm thông báo hoặc hiện tượng bạn gặp, rồi xem cách khắc phục. Một số thông báo đến từ máy chủ, Claude Code hoặc tiện ích trình duyệt và luôn hiện bằng tiếng Anh; chúng được giữ nguyên dưới đây.',
    keywords: [
      'lỗi',
      'sự cố',
      'không hoạt động',
      'không chạy',
      'khắc phục',
      'sửa lỗi',
      'không kết nối được',
    ],
    sections: [
      {
        heading: 'Kết nối',
        terms: [
          { term: 'Trực tiếp', text: 'Đã kết nối: cập nhật đến ngay khi có.' },
          {
            term: 'Hạn chế',
            text: 'Không mở được kênh cập nhật trực tiếp, nên ứng dụng tự làm mới vài giây một lần. Thường là vì điện thoại chưa tin chứng chỉ của máy tính: hãy tin cậy chứng chỉ đó, hoặc dùng địa chỉ đường hầm của bạn.',
          },
          {
            term: 'Không kết nối được máy Mac (hoặc máy PC)',
            text: 'Máy tính của bạn đang ngủ hoặc ngoại tuyến, đường hầm đã dừng, hoặc địa chỉ của đường hầm nhanh đã đổi. Hãy giữ máy tính thức khi bạn đi vắng.',
          },
          {
            term: 'Thiết bị này không còn được ghép nối.',
            text: 'Thiết bị đã bị thu hồi trong ứng dụng máy tính, lâu quá không dùng, hoặc dữ liệu lưu trên thiết bị đã bị xóa. Hãy ghép nối lại.',
          },
          {
            term: 'Trang cảnh báo trình duyệt của đường hầm đang chắn đường',
            text: 'Gói miễn phí của ngrok hiện một trang cảnh báo. Hãy mở địa chỉ đó một lần trong trình duyệt của điện thoại, chấp nhận cảnh báo rồi tải lại.',
          },
        ],
      },
      {
        heading: 'Ghép nối',
        terms: [
          {
            term: 'This code is for your home network and cannot be used through a tunnel or proxy.',
            text: 'Mã này chỉ dùng trong mạng nhà bạn. Bạn đã dùng mã **Cùng Wi-Fi** qua dữ liệu di động, đường hầm hoặc Tailscale. Hãy tạo mã **Internet**.',
          },
          {
            term: 'This code was already used by another device.',
            text: 'Mã đã được một thiết bị khác dùng. Hãy từ chối yêu cầu đó và tạo mã mới.',
          },
          {
            term: 'Those digits do not match the ones on the phone.',
            text: 'Các chữ số không khớp. Hãy nhập 4 chữ số mà điện thoại đang hiện. Sau ba lần sai, hãy bắt đầu lại trên điện thoại.',
          },
          {
            term: 'Chưa có URL công khai',
            text: 'Không tìm thấy đường hầm nào. Hãy chạy một trong các lệnh trong **Khởi chạy đường hầm** trên **Điện thoại & thiết bị**, hoặc nhập địa chỉ của đường hầm ở **Hoặc tự nhập**.',
          },
          {
            term: 'Mother Claude không lắng nghe trên mạng cục bộ của bạn.',
            text: 'Máy tính của bạn không có địa chỉ mạng riêng (hoặc ứng dụng được khởi động chỉ cho máy này). Hãy dùng đường hầm, hoặc bật **Lắng nghe cả trên Tailscale**.',
          },
        ],
      },
      {
        heading: 'Tin nhắn và phiên',
        terms: [
          {
            term: 'Tin nhắn chờ “sau bước hiện tại”',
            text: 'Claude đọc tin nhắn khi bước hiện tại kết thúc, trong cùng lượt. **Gửi ngay** để Claude đọc ngay lập tức.',
          },
          {
            term: 'Tin nhắn vẫn “trong hàng đợi”',
            text: 'Phiên (bắt đầu ở nơi khác) chưa tới lệnh gọi công cụ hay cuối lượt, hoặc không có gì đánh thức được nó. Bạn có thể rút tin nhắn khỏi hàng đợi nếu đổi ý. Nếu sau 30 phút vẫn chưa được đọc, tin nhắn bị bỏ và bạn sẽ nhận thông báo.',
          },
          {
            term: 'Phiên bắt đầu trước khi cài plugin Mother Claude',
            text: 'Gõ `/reload-plugins` trong phiên đó, hoặc khởi động lại nó.',
          },
          {
            term: '`session_waiting` (tmux)',
            text: 'Phiên đang hiện một hộp thoại. Hãy trả lời hộp thoại trước, hoặc mở chế độ xem terminal.',
          },
          {
            term: '`session_not_foreground` (tmux)',
            text: 'Claude không phải chương trình đang ở phía trước trong pane đó. Hãy đưa Claude Code trở lại phía trước trong terminal của nó.',
          },
          {
            term: '`session_ended`',
            text: 'Phiên đã kết thúc. Hãy tiếp tục nó ở đây.',
          },
          {
            term: '`untrusted_cwd`',
            text: 'Claude Code chưa từng được mở trong thư mục đó trên máy tính của bạn. Hãy mở Claude Code ở đó một lần, hoặc bắt đầu phiên từ ứng dụng máy tính.',
          },
          {
            term: '`dangerous_blocked`',
            text: 'Thao tác đó cần ứng dụng máy tính, hoặc một thiết bị được phép làm thao tác nguy hiểm trên mạng mà nó đang dùng.',
          },
        ],
      },
      {
        heading: 'Thông báo',
        points: [
          '“Bị chặn trong cài đặt trang web của trình duyệt”: hãy cho phép thông báo cho trang này.',
          'Trên iPhone: thêm Mother Claude vào Màn hình chính và bật thông báo đẩy từ đó.',
          'Trong mạng nhà bạn: hãy tin cậy chứng chỉ của máy tính trước.',
          '**Gửi thử** ở **Cài đặt › Thông báo** sẽ kiểm tra toàn bộ đường đi.',
        ],
      },
      {
        heading: 'Phiên bắt đầu ở nơi khác',
        terms: [
          {
            term: 'Turn on Sessions started elsewhere on the … to approve and message this session from here.',
            text: 'Tài khoản của phiên chưa có plugin Mother Claude. Trong ứng dụng máy tính, nhấn **Cài đặt** ở dòng của tài khoản đó trong **Phiên bắt đầu ở nơi khác**, rồi gõ `/reload-plugins` trong phiên.',
          },
          {
            term: '“Cài đặt” bị mờ',
            text: 'Không tìm thấy lệnh `claude`. Hãy cài Claude Code, hoặc mở ứng dụng từ một shell mà `claude` chạy được.',
          },
          {
            term: 'Phiên trong terminal không có công cụ Mother Claude',
            text: 'Hãy cài plugin Mother Claude cho tài khoản của phiên đó (**Cài đặt** ở dòng của tài khoản trong **Phiên bắt đầu ở nơi khác**), rồi gõ `/reload-plugins`.',
          },
        ],
      },
      {
        heading: 'Tài khoản và đăng nhập',
        terms: [
          {
            term: 'Trình duyệt dừng ở một trang hiện mã',
            text: 'Đó là điều bình thường: hãy dán mã vào **Mã từ trình duyệt** và nhấn **Gửi mã**.',
          },
          {
            term: 'Claude Code không nhận mã đó',
            text: 'Hãy dán toàn bộ mã. Mỗi mã chỉ dùng được một lần; muốn có mã mới, hãy đăng nhập lại.',
          },
          {
            term: 'Đăng nhập nhầm tài khoản Claude',
            text: 'Hãy đăng nhập lại trong cửa sổ Chrome ẩn danh, rồi nhấn **Giữ** trên cảnh báo khi đã đúng tài khoản.',
          },
          {
            term: 'Phiên trong terminal của tài khoản token lại hiện là Mặc định',
            text: 'Hãy cài plugin Mother Claude ở dòng của tài khoản Mặc định (dòng đầu tiên) trong **Phiên bắt đầu ở nơi khác**: phiên của tài khoản token dùng plugin của tài khoản Mặc định. Tài khoản của phiên sẽ hiện sau lệnh gọi công cụ đầu tiên của nó.',
          },
          {
            term: 'Không gỡ bỏ được tài khoản',
            text: 'Các phiên do Mother Claude khởi chạy hoặc API vẫn đang dùng nó. Hãy dừng chúng, hoặc khởi động API bằng tài khoản khác: **Mở HTTP API** đưa bạn tới đó.',
          },
        ],
      },
      {
        heading: 'Bản sao lưu',
        terms: [
          {
            term: 'Mất cụm mật khẩu',
            text: 'Không có cách nào lấy lại: không ai đọc được các bản sao lưu đã tạo bằng cụm mật khẩu đó.',
          },
          {
            term: 'Đổi cụm mật khẩu chưa xong',
            text: 'Nhấn **Đặt lại cụm mật khẩu…** và đặt nó (cụm cũ hoặc cụm mới).',
          },
          {
            term: 'Kéo về hoặc khôi phục từ iCloud bị chờ hoặc thất bại',
            text: 'Nhấp chuột phải vào thư mục sao lưu và chọn **Keep Downloaded** (luôn giữ bản tải về).',
          },
          {
            term: 'Phiên từ máy tính kia không nằm đúng dự án',
            text: 'Hãy thêm một cặp thư mục, rồi kéo về lại.',
          },
          {
            term: 'Cả hai máy tính cùng thay đổi một phiên',
            text: 'Cả hai bản đều được giữ. Dùng **Sao lưu buộc…** hoặc tùy chọn buộc trong **Kéo về…** để chọn bản nào được ưu tiên.',
          },
        ],
      },
      {
        heading: 'Tiện ích trình duyệt',
        terms: [
          {
            term: 'Mother Claude is not reachable',
            text: 'Không kết nối được Mother Claude. Hãy kiểm tra xem ứng dụng có đang chạy không. Trên trang tùy chọn của tiện ích, **Address** phải là `127.0.0.1` và **Port** là `6725`; rồi nhấn **Save and reconnect**.',
          },
          {
            term: 'The code is not right, or has expired',
            text: 'Mã sai hoặc đã hết hạn. Hãy hiện một mã mới và nhập đủ 8 ký tự.',
          },
          {
            term: 'The user pressed Stop',
            text: 'Phiên đã bị dừng. Nhấn **Resume** trong bảng bên (hoặc **Tiếp tục** trong ứng dụng), hoặc chờ một phút.',
          },
          {
            term: 'Nobody approved …',
            text: 'Không ai cho phép kịp. Hãy trả lời sớm hơn, hoặc thêm một quyền cho phép lâu dài.',
          },
          {
            term: 'Another connection took over',
            text: 'Một kết nối khác đã giành quyền. Nhấn **Reconnect now** trên trang tùy chọn của tiện ích.',
          },
        ],
      },
      {
        heading: 'HTTP API và công cụ',
        terms: [
          {
            term: '`bridge_not_started`',
            text: 'Mở **HTTP API** và nhấn **Khởi động API**.',
          },
          {
            term: 'Not logged in · Please run /login',
            text: 'Claude Code chưa đăng nhập. Hãy đăng nhập lại bằng `claude auth login` trong terminal.',
          },
          {
            term: 'Lượt bị treo, không có sự kiện nào',
            text: 'Claude đang chờ một yêu cầu duyệt: hãy trả lời thẻ yêu cầu đó.',
          },
          {
            term: '`ECONNREFUSED` với công cụ Mother Claude',
            text: 'Mother Claude chưa chạy khi phiên bắt đầu. Hãy mở ứng dụng, rồi kết nối lại trong `/mcp` hoặc khởi động lại phiên.',
          },
          {
            term: '`mcp_disabled`',
            text: 'Bật lại **Cấp công cụ của Mother Claude cho các phiên** trong **Cài đặt › Công cụ cho Claude Code**.',
          },
        ],
      },
    ],
  },
};
