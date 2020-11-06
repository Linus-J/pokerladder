using System;
using System.Collections.Generic;
using System.Linq;
using System.Text;
using System.Threading.Tasks;
using System.Net.Mail;
using System.Net;

namespace thepokerladder
{
    class EmailRegister
    {
        SmtpClient smtpClient;
        static string fromMail = "pokerladder@gmail.com";
        static string password = "REMOVED FOR PUBLIC DISPLAY";
        public EmailRegister()
        {
            smtpClient = new SmtpClient()
            {
                Host = "smtp.gmail.com",
                Port = 587,
                EnableSsl = true,
                DeliveryMethod = SmtpDeliveryMethod.Network,
                Credentials = new NetworkCredential(fromMail, password),
                Timeout = 2000
            };
        }
        public void Send(string body, string receiveAddress)
        {
            using (var message = new MailMessage(fromMail, receiveAddress)
            {
                Subject = "Register",
                Body = body,
            })
            {
                message.IsBodyHtml = true;
                smtpClient.Send(message);
            }
        }
    }
}
